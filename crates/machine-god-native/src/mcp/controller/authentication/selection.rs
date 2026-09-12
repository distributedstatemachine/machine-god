//! Exact human-command selection, independent of runtime publication success.

use super::super::{
    NativeMcpControllerError as Error, NativeMcpControllerFailure, NativeMcpStartupPhase,
    configuration,
    state::{Failure, Generation, Inner, WorkerReservation, failure, lock},
};
use crate::{
    conversation_lifecycle::LifecyclePermit,
    mcp::{
        auth::{
            CommandCustody, McpAuthBrowser, McpAuthChallenge, McpAuthError, McpAuthLease,
            McpAuthLogoutReceipt, NativeMcpAuthProfile, NativeMcpAuthService, SelectedConfig,
        },
        config::{MAX_SERVER_NAME_BYTES, McpTransportConfig},
        startup::NativeMcpStartup,
        store::NativeMcpConfigSnapshot,
    },
};
use futures_util::future::{Either, select};
use machine_god_core::{BoxFuture, CancellationToken};
use std::{
    sync::{Arc, Mutex, Weak, atomic::Ordering},
    time::{Duration, Instant},
};

/// Shared real conversation admission, retained through credential workers.
pub(crate) struct ControlFence(Option<LifecyclePermit>);
impl ControlFence {
    pub(crate) fn new(permit: LifecyclePermit) -> Arc<Self> {
        Arc::new(Self(Some(permit)))
    }
    fn check(&self) -> Result<(), McpAuthError> {
        if self.0.as_ref().is_none_or(LifecyclePermit::was_quiesced) {
            Err(McpAuthError::Cancelled)
        } else {
            Ok(())
        }
    }
}
impl Drop for ControlFence {
    fn drop(&mut self) {
        if let Some(permit) = self.0.take() {
            contain(|| drop(permit));
        }
    }
}

pub(in crate::mcp::controller) struct CommandOwner {
    inner: Weak<Inner>,
    fence: Arc<ControlFence>,
    reservation: WorkerReservation,
    cancellation: CancellationToken,
    deadline: Instant,
}
impl CommandCustody for CommandOwner {
    fn check(&self) -> Result<(), McpAuthError> {
        self.fence.check()?;
        let inner = self.inner.upgrade().ok_or(McpAuthError::Unavailable)?;
        inner
            .check(&self.cancellation, Some(self.deadline))
            .map_err(|error| match error.kind {
                Error::Deadline => McpAuthError::Deadline,
                Error::Cancelled => McpAuthError::Cancelled,
                _ => McpAuthError::Unavailable,
            })
    }
    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(self.cancellation.cancelled())
    }
}
impl Drop for CommandOwner {
    fn drop(&mut self) {
        contain(|| {
            self.reservation.0.cancellation.cancel();
        });
    }
}

pub(crate) struct Selection {
    pub(crate) server: Box<str>,
    pub(crate) owner_cancellation: CancellationToken,
    owner: Arc<CommandOwner>,
    startup: Arc<NativeMcpStartup>,
    historical: Option<Arc<NativeMcpStartup>>,
    profile: Arc<NativeMcpAuthProfile>,
    service: Arc<NativeMcpAuthService>,
}
impl Selection {
    pub(crate) fn check(&self) -> Result<(), McpAuthError> {
        self.owner.check()
    }
    fn selected(&self) -> Result<SelectedConfig, McpAuthError> {
        self.check()?;
        Ok(SelectedConfig {
            config: self
                .startup
                .authentication_config(&self.server)
                .map_err(|_| McpAuthError::Invalid)?,
            profile: self.profile.clone(),
        })
    }
    fn challenge(&self) -> Result<McpAuthChallenge, McpAuthError> {
        let Some(startup) = &self.historical else {
            return Ok(McpAuthChallenge::default());
        };
        let Some(observed) = startup
            .authentication_challenge(&self.server)
            .map_err(|_| McpAuthError::Limit)?
        else {
            return Ok(McpAuthChallenge::default());
        };
        let mut bytes = Vec::new();
        for header in observed.challenges() {
            let separator = usize::from(!bytes.is_empty());
            if bytes.len() + separator + header.len() > 16 * 1024 {
                return Err(McpAuthError::Limit);
            }
            if separator != 0 {
                bytes.push(b',');
            }
            bytes.extend_from_slice(header);
        }
        McpAuthChallenge::parse(&bytes)
    }
    pub(crate) async fn authenticate(
        &self,
        browser: &dyn McpAuthBrowser,
    ) -> Result<McpAuthLease, McpAuthError> {
        let selected = self.selected()?;
        self.service
            .authenticate_profile(
                &selected,
                &self.challenge()?,
                browser,
                &self.owner.cancellation,
                self.owner.deadline,
            )
            .await
    }
    pub(crate) async fn logout(&self) -> Result<McpAuthLogoutReceipt, McpAuthError> {
        self.service
            .logout_profile(
                &self.selected()?,
                &self.owner.cancellation,
                self.owner.deadline,
            )
            .await
    }
}

type Historical = Option<(Arc<NativeMcpConfigSnapshot>, Arc<NativeMcpStartup>)>;
type ResultSelection = Result<Selection, NativeMcpControllerFailure>;

pub(in crate::mcp::controller) fn prepare(
    inner: Weak<Inner>,
    server: String,
    fence: Arc<ControlFence>,
    cancellation: CancellationToken,
    deadline: Instant,
) -> BoxFuture<'static, ResultSelection> {
    Box::pin(async move {
        if server.is_empty()
            || server.len() > MAX_SERVER_NAME_BYTES
            || !server
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(failure(Error::Invalid));
        }
        let inner = inner.upgrade().ok_or_else(|| failure(Error::Closed))?;
        let (owner, historical) = reserve(&inner, fence, cancellation, deadline)?;
        let generation = owner.reservation.0.clone();
        let result = load(&inner, owner, server.into_boxed_str(), historical).await;
        result.map_err(|data| NativeMcpControllerFailure {
            data,
            generation: Some(generation),
        })
    })
}

fn reserve(
    inner: &Arc<Inner>,
    fence: Arc<ControlFence>,
    cancellation: CancellationToken,
    deadline: Instant,
) -> Result<(Arc<CommandOwner>, Historical), NativeMcpControllerFailure> {
    inner
        .check(&cancellation, Some(deadline))
        .map_err(|data| NativeMcpControllerFailure {
            data,
            generation: None,
        })?;
    fence.check().map_err(|_| failure(Error::Cancelled))?;
    // The latest exact observation survives failed-startup cleanup. These bytes
    // are historical data; loading revalidates their profile before reusing them.
    let historical = lock(&inner.state)
        .latest_observed
        .as_ref()
        .and_then(|generation| {
            lock(&generation.loaded)
                .as_ref()
                .map(|loaded| (loaded.snapshot.clone(), loaded.startup.clone()))
        });
    inner.prune();
    let mut state = lock(&inner.state);
    if state.closed {
        return Err(failure(Error::Closed));
    }
    if inner.settling.load(Ordering::Acquire)
        || state.catalog_handoff
        || state.running.is_some()
        || state.authenticating.upgrade().is_some()
    {
        return Err(failure(Error::Busy));
    }
    if state.generations.len() == inner.options.max_retained_generations {
        return Err(failure(Error::Limit));
    }
    let generation = Arc::new(Generation {
        phase: NativeMcpStartupPhase::All,
        cancellation: CancellationToken::new(),
        loaded: Mutex::default(),
        deferred: Mutex::default(),
        workers: std::sync::atomic::AtomicUsize::new(0),
    });
    let owner = Arc::new(CommandOwner {
        inner: Arc::downgrade(inner),
        fence,
        reservation: WorkerReservation::new(&generation),
        cancellation,
        deadline,
    });
    state.generations.push(generation);
    state.authenticating = Arc::downgrade(&owner);
    drop(state);
    Ok((owner, historical))
}

async fn load(
    inner: &Arc<Inner>,
    owner: Arc<CommandOwner>,
    server: Box<str>,
    historical: Historical,
) -> Result<Selection, Failure> {
    let options = &inner.options;
    let service = options
        .stored_authentication
        .clone()
        .ok_or(Error::Unavailable)?;
    let store = options.management.config_store();
    let worker_owner = owner.clone();
    let source = store.clone();
    let worker = options.workers.run(move || {
        // Returning custody with the value also covers unconsumed worker output.
        let result = (|| {
            worker_owner.check().map_err(|_| Error::Cancelled)?;
            let snapshot = Arc::new(source.load()?);
            let historical = historical.and_then(|(observed, startup)| {
                source.validate_unchanged(&observed).ok().map(|()| startup)
            });
            Ok::<_, Failure>((snapshot, historical))
        })();
        (worker_owner, result)
    });
    let until = options
        .startup
        .clock
        .now()
        .checked_add(Duration::from_secs(30))
        .ok_or(Error::Limit)?
        .min(owner.deadline);
    let stopped = async {
        select(
            owner.cancellation.cancelled(),
            Box::pin(select(
                options.startup.owner_cancellation.cancelled(),
                options.startup.clock.sleep_until(until),
            )),
        )
        .await;
    };
    let (_retained, (snapshot, historical)) =
        match select(Box::pin(worker), Box::pin(stopped)).await {
            Either::Left((result, _)) => {
                let (retained, result) = result.map_err(|_| Error::Unavailable)?;
                (retained, result?)
            }
            Either::Right(_) => {
                return Err(if options.startup.clock.now() >= until {
                    Error::Deadline
                } else {
                    Error::Cancelled
                }
                .into());
            }
        };
    owner.check().map_err(|_| Error::Cancelled)?;
    let selected = snapshot.config().server(&server).ok_or(Error::Invalid)?;
    if matches!(selected.transport(), McpTransportConfig::Stdio(_)) {
        return Err(Error::Invalid.into());
    }
    let startup =
        configuration::startup(options, &snapshot, owner.reservation.0.cancellation.clone())?;
    let profile = Arc::new(
        NativeMcpAuthProfile::new(
            store,
            snapshot,
            options.startup.owner_cancellation.clone(),
            owner.reservation.0.cancellation.clone(),
        )
        .with_command(owner.clone()),
    );
    Ok(Selection {
        server,
        owner_cancellation: options.startup.owner_cancellation.clone(),
        owner,
        startup,
        historical,
        profile,
        service,
    })
}

fn contain(operation: impl FnOnce()) {
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        std::mem::forget(payload);
    }
}
