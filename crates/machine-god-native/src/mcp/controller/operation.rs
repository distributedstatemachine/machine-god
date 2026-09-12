use super::{
    BoxFuture, CancellationToken, Instant, NativeMcpControllerError, NativeMcpControllerFailure,
    NativeMcpControllerOptions, NativeMcpControllerPublication, NativeMcpControllerReceipt,
    NativeMcpStartupPhase, Result,
    state::{
        CancelOnDrop, Failure, Generation, Inner, JobResult, Kind, MAX_DRAIN_BATCH,
        MAX_PEER_OBSERVATIONS, Receipt, Running, State, failure, lock,
    },
};
use futures_util::{
    FutureExt,
    future::{Either, select},
};
use std::sync::{Arc, Mutex, Weak, atomic::Ordering};

mod budget;
mod publish;
mod refresh;
#[cfg(test)]
mod tests;

pub(super) fn request(
    inner: Weak<Inner>,
    kind: Kind,
    cancellation: CancellationToken,
    deadline: Option<Instant>,
) -> BoxFuture<'static, Result<NativeMcpControllerReceipt>> {
    Box::pin(async move {
        let inner = inner
            .upgrade()
            .ok_or_else(|| failure(NativeMcpControllerError::Closed))?;
        let deadline = deadline
            .map_or(inner.options.startup.peer_lifetime.deadline(), |deadline| {
                Some(inner.options.startup.peer_lifetime.constrain(deadline))
            });
        inner
            .check(&cancellation, deadline)
            .map_err(|data| NativeMcpControllerFailure {
                data,
                generation: None,
            })?;
        inner.prune();
        let selected = select_job(&inner, kind, &cancellation, deadline)?;
        let (data, generation) = match selected {
            Selected::Complete(data, generation) => (data, generation),
            Selected::Running(job) => {
                let generation = job.generation.clone();
                let data = if matches!(kind, Kind::Deferred | Kind::Refresh) {
                    let stopped = async {
                        select(
                            cancellation.cancelled(),
                            Box::pin(budget::elapsed(&inner.options, deadline)),
                        )
                        .await;
                    };
                    match select(Box::pin(job.future), Box::pin(stopped)).await {
                        Either::Left((result, _)) => result,
                        Either::Right(_) => Err(if cancellation.is_cancelled() {
                            NativeMcpControllerError::Cancelled
                        } else {
                            NativeMcpControllerError::Deadline
                        }
                        .into()),
                    }
                } else {
                    // The sole mutation observer cancels abandonment, but awaits
                    // the real job result: a published receipt wins cancellation.
                    let _abandoned = CancelOnDrop(job.cancellation.clone());
                    job.future.await
                };
                (data, generation)
            }
        };
        data.map(|data| NativeMcpControllerReceipt {
            data,
            generation: generation.clone(),
        })
        .map_err(|data| NativeMcpControllerFailure {
            data,
            generation: Some(generation),
        })
    })
}

enum Selected {
    Complete(JobResult, Arc<Generation>),
    Running(Running),
}

fn select_job(
    inner: &Arc<Inner>,
    kind: Kind,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<Selected> {
    let refresh = (kind == Kind::Refresh)
        .then(|| refresh::observe(inner))
        .transpose()?;
    let mut state = lock(&inner.state);
    if state.closed {
        return Err(failure(NativeMcpControllerError::Closed));
    }
    if state.catalog_handoff {
        return Err(failure(NativeMcpControllerError::Busy));
    }
    if inner.settling.load(Ordering::Acquire) {
        return Err(failure(NativeMcpControllerError::Busy));
    }
    #[cfg(all(feature = "mcp-http", any(test, feature = "ai-gateway-http")))]
    if state.authenticating.upgrade().is_some() {
        return Err(failure(NativeMcpControllerError::Busy));
    }
    if matches!(kind, Kind::Start(NativeMcpStartupPhase::AskDeferred))
        || matches!(kind, Kind::Start(_)) && state.active.is_some()
    {
        return Err(failure(NativeMcpControllerError::Invalid));
    }
    if let Some(refresh) = &refresh {
        if state
            .active
            .as_ref()
            .is_none_or(|active| !Arc::ptr_eq(active, &refresh.source))
        {
            return Err(failure(NativeMcpControllerError::Busy));
        }
        if !refresh.due {
            return Ok(Selected::Complete(Ok(unchanged()), refresh.source.clone()));
        }
    }
    let deferred = if kind == Kind::Deferred {
        let generation = state
            .active
            .clone()
            .ok_or_else(|| failure(NativeMcpControllerError::Unavailable))?;
        if generation.phase == NativeMcpStartupPhase::All {
            return Ok(Selected::Complete(Ok(unchanged()), generation));
        }
        let completed = lock(&generation.deferred).clone();
        if let Some(completed) = completed {
            return Ok(Selected::Complete(completed, generation));
        }
        Some(generation)
    } else {
        None
    };
    if let Some(running) = &state.running {
        if kind == Kind::Refresh
            && running.kind == Kind::Refresh
            && refresh.as_ref().is_some_and(|observation| {
                running
                    .refresh_source
                    .as_ref()
                    .is_some_and(|source| Arc::ptr_eq(source, &observation.source))
            })
        {
            return Ok(Selected::Running(running.clone()));
        }
        if kind == Kind::Deferred
            && running.kind == Kind::Deferred
            && deferred
                .as_ref()
                .is_some_and(|generation| Arc::ptr_eq(generation, &running.generation))
        {
            return Ok(Selected::Running(running.clone()));
        }
        return Err(failure(NativeMcpControllerError::Busy));
    }
    // Reserve both this mutation's pre-start drain and the final close drain
    // before admitting effects; incomplete returned batches remain charged.
    if state.peers.len() > MAX_PEER_OBSERVATIONS - 2 * MAX_DRAIN_BATCH {
        return Err(failure(NativeMcpControllerError::Limit));
    }
    start_job(
        inner,
        &mut state,
        kind,
        deferred,
        refresh,
        cancellation,
        deadline,
    )
}

fn start_job(
    inner: &Arc<Inner>,
    state: &mut State,
    kind: Kind,
    deferred: Option<Arc<Generation>>,
    refresh: Option<refresh::Observation>,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<Selected> {
    let generation = if let Some(generation) = deferred {
        generation
    } else {
        if state.generations.len() == inner.options.max_retained_generations {
            return Err(failure(NativeMcpControllerError::Limit));
        }
        let generation = Arc::new(Generation {
            phase: match kind {
                Kind::Start(phase) => phase,
                Kind::Refresh => {
                    refresh::phase(&refresh.as_ref().expect("refresh observation").source)
                }
                _ => NativeMcpStartupPhase::All,
            },
            cancellation: CancellationToken::new(),
            loaded: Mutex::default(),
            deferred: Mutex::default(),
            workers: std::sync::atomic::AtomicUsize::new(0),
        });
        // This slot exists before store I/O or any polled startup effects.
        state.generations.push(generation.clone());
        generation
    };
    let job_cancel = CancellationToken::new();
    let signals = Signals {
        job: job_cancel.clone(),
        caller: (!matches!(kind, Kind::Deferred | Kind::Refresh)).then(|| cancellation.clone()),
    };
    let refresh_source = refresh.map(|observation| observation.source);
    let future = run(
        Arc::downgrade(inner),
        inner.options.clone(),
        generation.clone(),
        refresh_source.clone(),
        kind,
        signals,
        deadline,
    )
    .shared();
    let running = Running {
        kind,
        generation,
        refresh_source,
        cancellation: job_cancel,
        future,
    };
    state.running = Some(running.clone());
    Ok(Selected::Running(running))
}

#[derive(Clone)]
struct Signals {
    job: CancellationToken,
    caller: Option<CancellationToken>,
}
impl Signals {
    fn check(&self, inner: &Inner, deadline: Option<Instant>) -> std::result::Result<(), Failure> {
        inner.check(&self.job, deadline)?;
        if self
            .caller
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(NativeMcpControllerError::Cancelled.into());
        }
        Ok(())
    }
    async fn stopped(&self, options: &NativeMcpControllerOptions, deadline: Option<Instant>) {
        let caller = async {
            match &self.caller {
                Some(caller) => caller.cancelled().await,
                None => std::future::pending().await,
            }
        };
        let owner = async {
            select(
                self.job.cancelled(),
                options.startup.owner_cancellation.cancelled(),
            )
            .await;
        };
        select(
            Box::pin(select(Box::pin(caller), Box::pin(owner))),
            Box::pin(budget::elapsed(options, deadline)),
        )
        .await;
    }
}

fn run(
    inner: Weak<Inner>,
    options: Arc<NativeMcpControllerOptions>,
    generation: Arc<Generation>,
    refresh_source: Option<Arc<Generation>>,
    kind: Kind,
    signals: Signals,
    deadline: Option<Instant>,
) -> BoxFuture<'static, JobResult> {
    Box::pin(async move {
        let operation = async {
            let owner = inner.upgrade().ok_or(NativeMcpControllerError::Closed)?;
            signals.check(&owner, deadline)?;
            drop(owner);
            if kind == Kind::Deferred {
                publish::deferred(&inner, &options, &generation, &signals, deadline).await
            } else {
                publish::replace(
                    &inner,
                    &options,
                    &generation,
                    refresh_source.as_ref(),
                    &signals,
                    deadline,
                )
                .await
            }
        };
        // Observe an already-signalled cutoff before advancing another startup
        // stage. Publication has no following await, so once performed its
        // successful result still wins a cancellation triggered by publication.
        let result = match select(
            Box::pin(signals.stopped(&options, deadline)),
            Box::pin(operation),
        )
        .await
        {
            Either::Right((result, _)) => result,
            Either::Left(_) => Err(if deadline
                .is_some_and(|deadline| options.startup.clock.now() >= deadline)
            {
                NativeMcpControllerError::Deadline
            } else {
                NativeMcpControllerError::Cancelled
            }
            .into()),
        };
        if kind == Kind::Deferred {
            *lock(&generation.deferred) = Some(result.clone());
        } else if result.is_err() {
            generation.cancellation.cancel();
        }
        result
    })
}

fn check(
    inner: &Weak<Inner>,
    signals: &Signals,
    deadline: Option<Instant>,
) -> std::result::Result<Arc<Inner>, Failure> {
    let inner = inner.upgrade().ok_or(NativeMcpControllerError::Closed)?;
    signals.check(&inner, deadline)?;
    Ok(inner)
}
fn unchanged() -> Receipt {
    Receipt {
        startup: None,
        publication: NativeMcpControllerPublication::Unchanged,
        closed: false,
    }
}
