use super::super::{
    NativeMcpControllerError, NativeMcpControllerOptions, NativeMcpControllerPublication,
    NativeMcpStartupPhase,
    state::{
        Failure, Generation, Inner, JobResult, Kind, Loaded, Receipt, WorkerReservation, lock,
    },
};
use super::{Signals, budget, check, unchanged};
use crate::mcp::{
    startup::{NativeMcpStartup, NativeMcpStartupOptions, NativeMcpStartupRequirement},
    store::NativeMcpConfigSnapshot,
};
use machine_god_core::ToolName;
use std::{
    sync::{Arc, Weak},
    time::Instant,
};

pub(super) async fn replace(
    inner: &Weak<Inner>,
    options: &Arc<NativeMcpControllerOptions>,
    generation: &Arc<Generation>,
    kind: Kind,
    signals: &Signals,
    deadline: Option<Instant>,
) -> JobResult {
    drop(check(inner, signals, deadline)?);
    let expected = options.runtime.publication_checkpoint()?;
    // Returned observations keep their custody even if later profile I/O fails.
    let cleanup_deadline = budget::housekeeping_deadline(options, deadline)?;
    let completions = options
        .runtime
        .drain_retired(cleanup_deadline, signals.job.clone())
        .await?;
    let owner = inner.upgrade().ok_or(NativeMcpControllerError::Closed)?;
    lock(&owner.state).peers.extend(completions);
    signals.check(&owner, deadline)?;
    drop(owner);
    let store = options.management.config_store();
    let reservation = WorkerReservation::new(generation);
    let snapshot = Arc::new(
        budget::housekeeping(
            options,
            deadline,
            options.workers.run(move || {
                let _reservation = reservation;
                store.load()
            }),
        )
        .await?
        .map_err(|_| NativeMcpControllerError::Unavailable)??,
    );
    drop(check(inner, signals, deadline)?);
    let startup = selected_startup(options, generation, &snapshot)?;
    *lock(&generation.loaded) = Some(Loaded {
        snapshot: snapshot.clone(),
        startup: startup.clone(),
        checkpoint: None,
    });
    let batch = match deadline {
        Some(deadline) => {
            startup
                .build(generation.phase, signals.job.clone(), deadline)
                .await
        }
        None => {
            startup
                .build_configured(generation.phase, signals.job.clone())
                .await
        }
    };
    let reserved: Vec<_> = options
        .reserved_tool_names
        .iter()
        .map(ToolName::as_str)
        .collect();
    let requirement = if kind == Kind::Reload {
        NativeMcpStartupRequirement::AllSelected
    } else {
        NativeMcpStartupRequirement::Required
    };
    let (candidate, receipt) = batch.prepare(&options.runtime, &reserved, requirement)?;
    validate(options, generation, snapshot, deadline).await?;
    let owner = check(inner, signals, deadline)?;
    let predicted = candidate.publication_checkpoint();
    options.runtime.publish_if(candidate, &expected)?;
    // No await or fallible check after publication: an irrevocable publication
    // receipt cannot be erased by a concurrent caller cancellation or close.
    let (closed, previous) = {
        let mut state = lock(&owner.state);
        if let Some(loaded) = lock(&generation.loaded).as_mut() {
            loaded.checkpoint = Some(predicted);
        }
        let previous = if state.closed {
            None
        } else {
            state.active.replace(generation.clone())
        };
        (state.closed, previous)
    };
    if let Some(previous) = previous {
        previous.cancellation.cancel();
    }
    // Retiring the old owner may synchronously wake a caller which closes us.
    let closed = closed || lock(&owner.state).closed;
    Ok(Receipt {
        startup: Some(receipt),
        publication: NativeMcpControllerPublication::Published,
        closed,
    })
}

fn selected_startup(
    options: &NativeMcpControllerOptions,
    generation: &Generation,
    snapshot: &Arc<NativeMcpConfigSnapshot>,
) -> std::result::Result<Arc<NativeMcpStartup>, Failure> {
    let selected = &options.startup;
    Ok(Arc::new(NativeMcpStartup::new(NativeMcpStartupOptions {
        configuration: Arc::new(snapshot.config().clone()),
        captured_environment: selected.captured_environment.clone(),
        stdio: selected.stdio.clone(),
        workers: options.workers.clone(),
        clock: selected.clock.clone(),
        catalog_epoch: selected.catalog_epoch,
        owner_cancellation: selected.owner_cancellation.clone(),
        configuration_cancellation: generation.cancellation.clone(),
        #[cfg(feature = "mcp-http")]
        network: selected.network.clone(),
        #[cfg(feature = "mcp-http")]
        authentication: super::super::authentication::selections(
            options,
            snapshot,
            generation.cancellation.clone(),
        )?,
        peer_lifetime: selected.peer_lifetime,
        max_retained_bytes: selected.max_retained_bytes,
    })?))
}

pub(super) async fn deferred(
    inner: &Weak<Inner>,
    options: &Arc<NativeMcpControllerOptions>,
    generation: &Arc<Generation>,
    signals: &Signals,
    deadline: Option<Instant>,
) -> JobResult {
    drop(check(inner, signals, deadline)?);
    let (snapshot, startup, checkpoint) = {
        let loaded = lock(&generation.loaded);
        let loaded = loaded
            .as_ref()
            .ok_or(NativeMcpControllerError::Unavailable)?;
        (
            loaded.snapshot.clone(),
            loaded.startup.clone(),
            loaded
                .checkpoint
                .clone()
                .ok_or(NativeMcpControllerError::Unavailable)?,
        )
    };
    if !snapshot
        .config()
        .servers()
        .iter()
        .any(|server| server.enabled() && !server.required())
    {
        return Ok(unchanged());
    }
    validate(options, generation, snapshot.clone(), deadline).await?;
    drop(check(inner, signals, deadline)?);
    let batch = match deadline {
        Some(deadline) => {
            startup
                .build(
                    NativeMcpStartupPhase::AskDeferred,
                    signals.job.clone(),
                    deadline,
                )
                .await
        }
        None => {
            startup
                .build_configured(NativeMcpStartupPhase::AskDeferred, signals.job.clone())
                .await
        }
    };
    let (servers, receipt) = batch.into_deferred_servers()?;
    validate(options, generation, snapshot, deadline).await?;
    let owner = check(inner, signals, deadline)?;
    if servers.is_empty() {
        return Ok(Receipt {
            startup: Some(receipt),
            ..unchanged()
        });
    }
    let reserved: Vec<_> = options
        .reserved_tool_names
        .iter()
        .map(ToolName::as_str)
        .collect();
    let addition = options
        .runtime
        .prepare_addition(servers, &reserved, &checkpoint)?;
    let predicted = addition.publication_checkpoint();
    options.runtime.publish_addition(addition)?;
    let closed = {
        let state = lock(&owner.state);
        if let Some(loaded) = lock(&generation.loaded).as_mut() {
            loaded.checkpoint = Some(predicted);
        }
        state.closed
    };
    Ok(Receipt {
        startup: Some(receipt),
        publication: NativeMcpControllerPublication::Published,
        closed,
    })
}

async fn validate(
    options: &NativeMcpControllerOptions,
    generation: &Arc<Generation>,
    snapshot: Arc<NativeMcpConfigSnapshot>,
    deadline: Option<Instant>,
) -> std::result::Result<(), Failure> {
    let store = options.management.config_store();
    let reservation = WorkerReservation::new(generation);
    budget::housekeeping(
        options,
        deadline,
        options.workers.run(move || {
            let _reservation = reservation;
            store.validate_unchanged(&snapshot)
        }),
    )
    .await?
    .map_err(|_| NativeMcpControllerError::Unavailable)??;
    Ok(())
}
