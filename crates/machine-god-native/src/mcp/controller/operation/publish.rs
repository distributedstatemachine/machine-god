use super::super::{
    NativeMcpControllerError, NativeMcpControllerOptions, NativeMcpControllerPublication,
    NativeMcpStartupPhase,
    state::{
        Failure, Generation, Inner, JobResult, Kind, Loaded, Receipt, WorkerReservation, lock,
    },
};
use super::{Signals, check, unchanged};
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
    deadline: Instant,
) -> JobResult {
    drop(check(inner, signals, deadline)?);
    let expected = options.runtime.publication_checkpoint()?;
    // Returned observations keep their custody even if later profile I/O fails.
    let completions = options
        .runtime
        .drain_retired(deadline, signals.job.clone())
        .await?;
    let owner = inner.upgrade().ok_or(NativeMcpControllerError::Closed)?;
    lock(&owner.state).peers.extend(completions);
    signals.check(&owner, deadline)?;
    drop(owner);
    let store = options.management.config_store();
    let reservation = WorkerReservation::new(generation);
    let snapshot = Arc::new(
        options
            .workers
            .run(move || {
                let _reservation = reservation;
                store.load()
            })
            .await
            .map_err(|_| NativeMcpControllerError::Unavailable)??,
    );
    drop(check(inner, signals, deadline)?);
    let selected = &options.startup;
    let startup = Arc::new(NativeMcpStartup::new(NativeMcpStartupOptions {
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
        authentication: selected.authentication.clone(),
        peer_lifetime_deadline: selected.peer_lifetime_deadline,
        max_retained_bytes: selected.max_retained_bytes,
    })?);
    *lock(&generation.loaded) = Some(Loaded {
        snapshot: snapshot.clone(),
        startup: startup.clone(),
        checkpoint: None,
    });
    let batch = startup
        .build(generation.phase, signals.job.clone(), deadline)
        .await;
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
    validate(options, generation, snapshot).await?;
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

pub(super) async fn deferred(
    inner: &Weak<Inner>,
    options: &Arc<NativeMcpControllerOptions>,
    generation: &Arc<Generation>,
    signals: &Signals,
    deadline: Instant,
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
    validate(options, generation, snapshot.clone()).await?;
    drop(check(inner, signals, deadline)?);
    let batch = startup
        .build(
            NativeMcpStartupPhase::AskDeferred,
            signals.job.clone(),
            deadline,
        )
        .await;
    let (servers, receipt) = batch.into_deferred_servers()?;
    validate(options, generation, snapshot).await?;
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
) -> std::result::Result<(), Failure> {
    let store = options.management.config_store();
    let reservation = WorkerReservation::new(generation);
    options
        .workers
        .run(move || {
            let _reservation = reservation;
            store.validate_unchanged(&snapshot)
        })
        .await
        .map_err(|_| NativeMcpControllerError::Unavailable)??;
    Ok(())
}
