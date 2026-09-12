use super::{
    BoxFuture, CancellationToken, Instant, NativeMcpControllerCleanup, NativeMcpControllerError,
    NativeMcpControllerFailure, Result,
    state::{Failure, Inner, Settlement, cleanup_check, failure, lock},
};
use futures_util::future::{Either, select};
use std::{
    sync::{Arc, Weak, atomic::Ordering},
    time::Duration,
};

pub(super) fn settle(
    inner: Weak<Inner>,
    deadline: Instant,
    cancellation: CancellationToken,
) -> BoxFuture<'static, Result<NativeMcpControllerCleanup>> {
    Box::pin(async move {
        let inner = inner
            .upgrade()
            .ok_or_else(|| failure(NativeMcpControllerError::Closed))?;
        inner
            .settling
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| failure(NativeMcpControllerError::Busy))?;
        let _settlement = Settlement(inner.clone());
        inner.close();
        let result = settle_all(&inner, deadline, cancellation).await;
        result.map_err(|data| NativeMcpControllerFailure {
            data,
            generation: None,
        })
    })
}

async fn settle_all(
    inner: &Arc<Inner>,
    deadline: Instant,
    cancellation: CancellationToken,
) -> std::result::Result<NativeMcpControllerCleanup, Failure> {
    let peers = run(inner, deadline, cancellation.clone());
    #[cfg(feature = "mcp-http")]
    {
        // Poll both retained owners even when one fails. Neither creates a task
        // nor closes the host worker scope. Caller-owned auth network/browser
        // futures must already have completed or been dropped by their host.
        let authentication = async {
            let Some(service) = &inner.options.stored_authentication else {
                return Ok::<(), Failure>(());
            };
            let receipt = service
                .settle(deadline, cancellation)
                .await
                .map_err(|_| Failure::from(NativeMcpControllerError::Unavailable))?;
            if !receipt.complete {
                return Err(NativeMcpControllerError::Unavailable.into());
            }
            Ok(())
        };
        let (peers, authentication) = futures_util::future::join(peers, authentication).await;
        authentication?;
        peers
    }
    #[cfg(not(feature = "mcp-http"))]
    peers.await
}

async fn run(
    inner: &Arc<Inner>,
    deadline: Instant,
    cancellation: CancellationToken,
) -> std::result::Result<NativeMcpControllerCleanup, Failure> {
    cleanup_check(&inner.options, &cancellation, deadline)?;
    let running = lock(&inner.state).running.clone();
    if let Some(running) = running {
        let stopped = async {
            select(
                cancellation.cancelled(),
                inner.options.startup.clock.sleep_until(deadline),
            )
            .await;
        };
        if let Either::Right(_) = select(Box::pin(running.future), Box::pin(stopped)).await {
            cleanup_check(&inner.options, &cancellation, deadline)?;
            return Err(NativeMcpControllerError::Deadline.into());
        }
    }
    inner.release_completed();
    let completions = inner
        .options
        .runtime
        .drain_retired(deadline, cancellation.clone())
        .await?;
    lock(&inner.state).peers.extend(completions);
    loop {
        inner.prune();
        let receipt = {
            let state = lock(&inner.state);
            let pending_generations = state
                .generations
                .iter()
                .filter(|value| !value.cleanup_complete())
                .count();
            let pending_peers = state.peers.len();
            NativeMcpControllerCleanup {
                complete: pending_generations == 0 && pending_peers == 0,
                pending_generations,
                pending_peers,
            }
        };
        if receipt.complete {
            return Ok(receipt);
        }
        cleanup_check(&inner.options, &cancellation, deadline)?;
        let until = (inner.options.startup.clock.now() + Duration::from_millis(5)).min(deadline);
        select(
            cancellation.cancelled(),
            inner.options.startup.clock.sleep_until(until),
        )
        .await;
    }
}
