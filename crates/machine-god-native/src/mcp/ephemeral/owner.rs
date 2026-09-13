use super::{
    NativeMcpEphemeralConfiguration, NativeMcpEphemeralError as Error, NativeMcpEphemeralOptions,
};
use crate::{
    background_process::ValidatedBackgroundEnvironment,
    mcp::{
        runtime::{NativeMcpPeerCompletion, NativeMcpPublicationCheckpoint},
        startup::{
            NativeMcpStartup, NativeMcpStartupCompletion, NativeMcpStartupOptions,
            NativeMcpStartupPhase, NativeMcpStartupReceipt, NativeMcpStartupRequirement,
        },
    },
};
use machine_god_core::{BoxFuture, CancellationToken, ToolName};
use std::{
    fmt,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Error>;
mod catalog;
pub(crate) use catalog::NativeMcpEphemeralCatalog;

struct Generation {
    configuration: NativeMcpEphemeralConfiguration,
    cancellation: CancellationToken,
    startup: Arc<NativeMcpStartup>,
}
impl Generation {
    fn cleanup_complete(&self) -> bool {
        self.startup
            .cleanup_observations()
            .iter()
            .all(NativeMcpStartupCompletion::is_complete)
    }
}
struct Active {
    generation: Arc<Generation>,
    checkpoint: NativeMcpPublicationCheckpoint,
}
struct State {
    closed: bool,
    mutation: MutationState,
    settling: bool,
    initial: NativeMcpPublicationCheckpoint,
    active: Option<Active>,
    generations: Vec<Arc<Generation>>,
    peers: Vec<NativeMcpPeerCompletion>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum MutationState {
    Idle,
    Replacement,
    Catalog,
}

/// One mutation at a time, with cleanup custody installed before startup's first
/// poll. Receipt retention counts against the generation limit.
pub struct NativeMcpEphemeralOwner {
    options: NativeMcpEphemeralOptions,
    state: Arc<Mutex<State>>,
    catalog: Arc<NativeMcpEphemeralCatalog>,
}
/// Point-in-time successful publication, not a reusable execution capability.
#[derive(Clone)]
pub struct NativeMcpEphemeralReceipt {
    startup: NativeMcpStartupReceipt,
    generation: Arc<Generation>,
    closed_after_publication: bool,
}
impl NativeMcpEphemeralReceipt {
    #[must_use]
    pub fn startup(&self) -> &NativeMcpStartupReceipt {
        &self.startup
    }
    #[must_use]
    pub fn closed_after_publication(&self) -> bool {
        self.closed_after_publication
    }
    #[must_use]
    pub fn cleanup_complete(&self) -> bool {
        self.generation.cleanup_complete()
    }
}

impl NativeMcpEphemeralOwner {
    /// Validates captured selections without clock, worker, filesystem, process,
    /// network or credential effects. Existing publications are rejected.
    /// # Errors
    /// Invalid bounds/environment, closed runtime or preexisting publication.
    pub fn new(options: NativeMcpEphemeralOptions) -> Result<Self> {
        if !(1..=8).contains(&options.max_retained_generations)
            || !(1..=256 * 1024 * 1024).contains(&options.max_retained_bytes)
            || options.reserved_tool_names.len() > 4096
        {
            return Err(Error::Limit);
        }
        let mut names = std::collections::BTreeSet::new();
        if options
            .reserved_tool_names
            .iter()
            .any(|name| !names.insert(name.as_str()))
        {
            return Err(Error::Invalid);
        }
        ValidatedBackgroundEnvironment::new(options.captured_environment.clone())
            .map_err(|_| Error::Invalid)?;
        let initial = options.runtime.publication_checkpoint()?;
        if !initial.is_unpublished() {
            return Err(Error::Invalid);
        }
        let state = Arc::new(Mutex::new(State {
            closed: false,
            mutation: MutationState::Idle,
            settling: false,
            initial,
            active: None,
            generations: Vec::new(),
            peers: Vec::new(),
        }));
        let catalog = Arc::new(NativeMcpEphemeralCatalog::new(
            state.clone(),
            options.owner_cancellation.clone(),
        ));
        options.runtime.bind_ephemeral(&catalog)?;
        Ok(Self {
            options,
            state,
            catalog,
        })
    }

    /// Privately builds every required server, then atomically replaces this
    /// session's exact previous publication. Omission is an empty replacement,
    /// never a profile fallback. Creating this future is inert.
    /// # Errors
    /// Failed readiness, stale publication, cancellation and finite ownership
    /// limits leave the old selection intact. Failed candidates remain owned.
    #[must_use]
    pub fn replace(
        &self,
        configuration: NativeMcpEphemeralConfiguration,
        cancellation: CancellationToken,
        deadline: Instant,
    ) -> BoxFuture<'_, Result<NativeMcpEphemeralReceipt>> {
        Box::pin(async move {
            self.check(&cancellation, deadline, false)?;
            self.prune();
            let generation = self.generation(configuration)?;
            let expected = {
                let mut state = lock(&self.state);
                if state.closed {
                    return Err(Error::Closed);
                }
                if state.mutation != MutationState::Idle || state.settling {
                    return Err(Error::Busy);
                }
                if state.generations.len() >= self.options.max_retained_generations {
                    return Err(Error::Limit);
                }
                state.mutation = MutationState::Replacement;
                state.generations.push(generation.clone());
                state
                    .active
                    .as_ref()
                    .map_or_else(|| state.initial.clone(), |active| active.checkpoint.clone())
            };
            let mut mutation = Mutation {
                owner: self,
                generation: generation.clone(),
                published: false,
            };
            self.drain(&cancellation, deadline).await?;
            self.check(&cancellation, deadline, false)?;
            let batch = generation
                .startup
                .build(NativeMcpStartupPhase::All, cancellation.clone(), deadline)
                .await;
            let reserved: Vec<_> = self
                .options
                .reserved_tool_names
                .iter()
                .map(ToolName::as_str)
                .collect();
            let (candidate, receipt) = batch
                .prepare(
                    &self.options.runtime,
                    &reserved,
                    NativeMcpStartupRequirement::AllSelected,
                )
                .map_err(|failure| Error::Startup(failure.error))?;
            let checkpoint = candidate.publication_checkpoint();
            self.check(&cancellation, deadline, false)?;
            // No arbitrary callbacks occur after the final cancellation check:
            // publish_if validates immutable guards and exact publication under
            // its own lock, then completes invalidations outside that lock.
            self.options.runtime.publish_if(candidate, &expected)?;
            mutation.published = true;
            let (previous, closed) = {
                let mut state = lock(&self.state);
                let closed = state.closed || self.options.owner_cancellation.is_cancelled();
                let previous = if closed {
                    None
                } else {
                    state.active.replace(Active {
                        generation: generation.clone(),
                        checkpoint,
                    })
                };
                (previous, closed)
            };
            if let Some(previous) = previous {
                previous.generation.cancellation.cancel();
            }
            if closed {
                self.close();
            }
            Ok(NativeMcpEphemeralReceipt {
                startup: receipt,
                generation,
                closed_after_publication: closed,
            })
        })
    }

    /// Required readiness for the owner's exact current publication.
    /// # Errors
    /// Absent, closed, superseded or unavailable required peers are rejected.
    pub fn ready(&self) -> Result<()> {
        {
            let state = lock(&self.state);
            if state.closed || self.options.owner_cancellation.is_cancelled() {
                return Err(Error::Closed);
            }
            if state.mutation == MutationState::Catalog {
                return Err(Error::Busy);
            }
        }
        self.catalog
            .required_readiness(&self.options.runtime)
            .map_err(Error::Runtime)
    }

    /// Irrevocable session-incarnation cutoff. This is not a cleanup receipt.
    pub fn close(&self) {
        let (generations, active) = {
            let mut state = lock(&self.state);
            state.closed = true;
            (state.generations.clone(), state.active.take())
        };
        self.options.owner_cancellation.cancel();
        for generation in generations {
            generation.cancellation.cancel();
        }
        self.options.runtime.close();
        drop(active);
    }

    /// Closes and observes owned peers with an independent cleanup token. Caller
    /// must poll or drop a concurrently held replacement future; no detached task
    /// is created to run it. Timeout retains all custody for another settlement.
    /// The host still owns worker-scope shutdown/join.
    /// # Errors
    /// Cleanup cancellation/deadline or a concurrent settlement.
    #[must_use]
    pub fn settle(
        &self,
        cancellation: CancellationToken,
        deadline: Instant,
    ) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            {
                let mut state = lock(&self.state);
                if state.settling {
                    return Err(Error::Busy);
                }
                state.settling = true;
            }
            let _settlement = Settlement(self);
            self.close();
            loop {
                self.check(&cancellation, deadline, true)?;
                self.drain(&cancellation, deadline).await?;
                self.prune();
                let complete = {
                    let state = lock(&self.state);
                    state.mutation == MutationState::Idle
                        && state.peers.is_empty()
                        && state
                            .generations
                            .iter()
                            .all(|generation| generation.cleanup_complete())
                };
                if complete {
                    return Ok(());
                }
                let until = self
                    .options
                    .clock
                    .now()
                    .checked_add(Duration::from_millis(5))
                    .ok_or(Error::Limit)?
                    .min(deadline);
                futures_util::future::select(
                    cancellation.cancelled(),
                    self.options.clock.sleep_until(until),
                )
                .await;
            }
        })
    }

    fn generation(
        &self,
        configuration: NativeMcpEphemeralConfiguration,
    ) -> Result<Arc<Generation>> {
        let cancellation = CancellationToken::new();
        let startup = Arc::new(NativeMcpStartup::new_ephemeral(
            NativeMcpStartupOptions {
                configuration: configuration.configuration.clone(),
                captured_environment: self.options.captured_environment.clone(),
                stdio: self.options.stdio.clone(),
                workers: self.options.workers.clone(),
                clock: self.options.clock.clone(),
                catalog_epoch: self.options.catalog_epoch,
                owner_cancellation: self.options.owner_cancellation.clone(),
                configuration_cancellation: cancellation.clone(),
                #[cfg(feature = "mcp-http")]
                network: self.options.network.clone(),
                #[cfg(feature = "mcp-http")]
                authentication: configuration
                    .headers
                    .iter()
                    .map(
                        |(server, headers)| crate::mcp::startup::NativeMcpStartupAuthentication {
                            server: server.clone(),
                            additional_headers: headers.clone(),
                            source: crate::mcp::startup::NativeMcpStartupAuthSource::Configured,
                        },
                    )
                    .collect(),
                peer_lifetime: self.options.peer_lifetime,
                max_retained_bytes: self.options.max_retained_bytes,
            },
            configuration.identities.clone(),
        )?);
        Ok(Arc::new(Generation {
            configuration,
            cancellation,
            startup,
        }))
    }

    async fn drain(&self, cancellation: &CancellationToken, deadline: Instant) -> Result<()> {
        let peers = self
            .options
            .runtime
            .drain_retired(deadline, cancellation.clone())
            .await?;
        let mut state = lock(&self.state);
        state.peers.retain(|peer| !peer.is_complete());
        // At most eight retained generations, each with at most 64 peers.
        // Runtime independently refuses excess retired peers before publication.
        state
            .peers
            .extend(peers.into_iter().filter(|peer| !peer.is_complete()));
        Ok(())
    }
    fn check(
        &self,
        cancellation: &CancellationToken,
        deadline: Instant,
        cleanup: bool,
    ) -> Result<()> {
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if !cleanup && self.options.owner_cancellation.is_cancelled() {
            return Err(Error::Closed);
        }
        if self.options.clock.now() >= deadline {
            return Err(Error::Deadline);
        }
        // Selected clocks may reenter cancellation/closure.
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if !cleanup && self.options.owner_cancellation.is_cancelled() {
            return Err(Error::Closed);
        }
        Ok(())
    }
    fn prune(&self) {
        let removed = {
            let mut state = lock(&self.state);
            state.peers.retain(|peer| !peer.is_complete());
            let mut removed = Vec::new();
            let mut index = 0;
            while index < state.generations.len() {
                let generation = &state.generations[index];
                if Arc::strong_count(generation) == 1
                    && generation.cancellation.is_cancelled()
                    && generation.cleanup_complete()
                {
                    removed.push(state.generations.remove(index));
                } else {
                    index += 1;
                }
            }
            removed
        };
        drop(removed);
    }
}
impl Drop for NativeMcpEphemeralOwner {
    fn drop(&mut self) {
        self.close();
    }
}
struct Mutation<'a> {
    owner: &'a NativeMcpEphemeralOwner,
    generation: Arc<Generation>,
    published: bool,
}
impl Drop for Mutation<'_> {
    fn drop(&mut self) {
        if !self.published {
            self.generation.cancellation.cancel();
        }
        lock(&self.owner.state).mutation = MutationState::Idle;
    }
}
struct Settlement<'a>(&'a NativeMcpEphemeralOwner);
impl Drop for Settlement<'_> {
    fn drop(&mut self) {
        lock(&self.0.state).settling = false;
    }
}
fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl fmt::Debug for NativeMcpEphemeralOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeMcpEphemeralOwner { <redacted> }")
    }
}
impl fmt::Debug for NativeMcpEphemeralReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeMcpEphemeralReceipt { <redacted> }")
    }
}
