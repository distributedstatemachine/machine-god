//! Owned single-flight caching of bounded, capability-bearing Gateway catalogs.

use std::fmt;
use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Poll, Waker};

use machine_god_core::{BoxFuture, CancellationToken, ProviderErrorKind};

use crate::{AiGatewayModelCatalogProvider, NativeModelCatalog};

/// Minimum interval between automatic failed-catalog attempts, in milliseconds.
pub const NATIVE_MODEL_CATALOG_RETRY_MS: u64 = 1_000;
/// Maximum simultaneously registered loading waiters per cache.
pub const NATIVE_MODEL_CATALOG_MAX_WAITERS: usize = 64;

/// Observable catalog loading phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeModelCatalogCacheState {
    Idle,
    Loading,
    Ready,
    Failed,
}

/// Data-free fetch failure; provider-controlled messages are not retained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeModelCatalogCacheFailure {
    pub kind: ProviderErrorKind,
    pub retryable: bool,
}

/// Immutable cache observation, sharing the bounded catalog without copying it.
#[derive(Clone)]
pub struct NativeModelCatalogCacheSnapshot {
    pub state: NativeModelCatalogCacheState,
    pub catalog: Option<Arc<NativeModelCatalog>>,
    pub last_failure: Option<NativeModelCatalogCacheFailure>,
    pub last_attempt_ms: Option<u64>,
}

impl fmt::Debug for NativeModelCatalogCacheSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeModelCatalogCacheSnapshot")
            .field("state", &self.state)
            .field("has_catalog", &self.catalog.is_some())
            .field("last_failure", &self.last_failure)
            .field("last_attempt_ms", &self.last_attempt_ms)
            .finish()
    }
}

/// Local waiting failed; provider failures are represented in the snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeModelCatalogCacheError {
    Cancelled,
    WaiterLimit,
}

impl fmt::Display for NativeModelCatalogCacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Cancelled => "model catalog waiting cancelled",
            Self::WaiterLimit => "model catalog waiter limit exceeded",
        })
    }
}

impl std::error::Error for NativeModelCatalogCacheError {}

struct CacheInner {
    snapshot: NativeModelCatalogCacheSnapshot,
    waiters: [Option<Arc<Waker>>; NATIVE_MODEL_CATALOG_MAX_WAITERS],
}

/// A cache bound to one immutable provider/access configuration.
///
/// No background task is spawned. The first polled eligible load owns the
/// provider future; joining callers only wait. The host must keep polling the
/// owner, or drop it to cancel the fetch and restore its preceding state.
pub struct NativeModelCatalogCache {
    provider: Arc<AiGatewayModelCatalogProvider>,
    inner: Mutex<CacheInner>,
}

impl fmt::Debug for NativeModelCatalogCache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeModelCatalogCache")
            .field("snapshot", &self.snapshot())
            .finish_non_exhaustive()
    }
}

impl NativeModelCatalogCache {
    #[must_use]
    pub fn new(provider: Arc<AiGatewayModelCatalogProvider>) -> Self {
        Self {
            provider,
            inner: Mutex::new(CacheInner {
                snapshot: NativeModelCatalogCacheSnapshot {
                    state: NativeModelCatalogCacheState::Idle,
                    catalog: None,
                    last_failure: None,
                    last_attempt_ms: None,
                },
                waiters: std::array::from_fn(|_| None),
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, CacheInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[must_use]
    pub fn snapshot(&self) -> NativeModelCatalogCacheSnapshot {
        self.lock().snapshot.clone()
    }

    /// Loads when idle, or when the pinned retry policy permits another attempt.
    /// Ready success has no TTL. `now_ms` is an explicitly supplied monotonic
    /// time at admission; callers should promptly poll this inert future.
    #[must_use]
    pub fn load(
        &self,
        now_ms: u64,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<NativeModelCatalogCacheSnapshot, NativeModelCatalogCacheError>> {
        self.fetch(now_ms, cancellation, false)
    }

    /// Explicitly requests a refresh, bypassing cooldown but never single-flight.
    #[must_use]
    pub fn refresh(
        &self,
        now_ms: u64,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<NativeModelCatalogCacheSnapshot, NativeModelCatalogCacheError>> {
        self.fetch(now_ms, cancellation, true)
    }

    fn fetch(
        &self,
        now_ms: u64,
        cancellation: CancellationToken,
        force: bool,
    ) -> BoxFuture<'_, Result<NativeModelCatalogCacheSnapshot, NativeModelCatalogCacheError>> {
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(NativeModelCatalogCacheError::Cancelled);
            }
            let previous = {
                let mut inner = self.lock();
                let state = &inner.snapshot;
                if state.state == NativeModelCatalogCacheState::Loading {
                    None
                } else {
                    let elapsed = state
                        .last_attempt_ms
                        .and_then(|last| now_ms.checked_sub(last));
                    let retry_due = elapsed
                        .is_some_and(|elapsed| elapsed >= NATIVE_MODEL_CATALOG_RETRY_MS)
                        && (state.state == NativeModelCatalogCacheState::Failed
                            || state.last_failure.is_some_and(|failure| failure.retryable));
                    if !force && state.state != NativeModelCatalogCacheState::Idle && !retry_due {
                        return Ok(state.clone());
                    }
                    let previous = state.clone();
                    inner.snapshot.state = NativeModelCatalogCacheState::Loading;
                    inner.snapshot.last_attempt_ms = Some(now_ms);
                    Some(previous)
                }
            };
            let Some(previous) = previous else {
                return self.wait_while_loading(cancellation).await;
            };
            let mut owner = LoadOwner {
                cache: self,
                previous: Some(previous),
            };
            let result = self.provider.list_model_details(cancellation.clone()).await;
            let snapshot = {
                let mut inner = self.lock();
                let state = &mut inner.snapshot;
                match result {
                    Ok(catalog) => {
                        if !catalog.entries().is_empty()
                            || state
                                .catalog
                                .as_ref()
                                .is_none_or(|old| old.entries().is_empty())
                        {
                            state.catalog = Some(Arc::new(catalog));
                        }
                        state.last_failure = None;
                        state.state = NativeModelCatalogCacheState::Ready;
                    }
                    Err(error) => {
                        state.last_failure = Some(NativeModelCatalogCacheFailure {
                            kind: error.kind,
                            retryable: error.retryable,
                        });
                        state.state = if state
                            .catalog
                            .as_ref()
                            .is_some_and(|old| !old.entries().is_empty())
                        {
                            NativeModelCatalogCacheState::Ready
                        } else {
                            NativeModelCatalogCacheState::Failed
                        };
                    }
                }
                state.clone()
            };
            owner.previous = None;
            self.wake_waiters();
            if cancellation.is_cancelled() {
                Err(NativeModelCatalogCacheError::Cancelled)
            } else {
                Ok(snapshot)
            }
        })
    }

    /// Waits only while a fetch is loading. Does not initiate a fetch or infer
    /// capabilities when unavailable. Dropping/cancelling a waiter leaves its
    /// owner and other waiters untouched.
    #[must_use]
    pub fn wait_while_loading(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<NativeModelCatalogCacheSnapshot, NativeModelCatalogCacheError>> {
        Box::pin(async move {
            let mut registration = Waiter {
                cache: self,
                slot: None,
            };
            let mut cancelled = pin!(cancellation.cancelled());
            poll_fn(|context| {
                if cancelled.as_mut().poll(context).is_ready() {
                    return Poll::Ready(Err(NativeModelCatalogCacheError::Cancelled));
                }
                let incoming = Arc::new(context.waker().clone());
                let mut inner = self.lock();
                if inner.snapshot.state != NativeModelCatalogCacheState::Loading {
                    return Poll::Ready(Ok(inner.snapshot.clone()));
                }
                let slot = if let Some(slot) = registration.slot {
                    slot
                } else {
                    let Some(slot) = inner.waiters.iter().position(Option::is_none) else {
                        return Poll::Ready(Err(NativeModelCatalogCacheError::WaiterLimit));
                    };
                    registration.slot = Some(slot);
                    slot
                };
                let old = inner.waiters[slot].replace(incoming);
                drop(inner);
                drop(old);
                Poll::Pending
            })
            .await
        })
    }

    fn wake_waiters(&self) {
        let waiters = self.lock().waiters.clone();
        for waiter in waiters.into_iter().flatten() {
            waiter.wake_by_ref();
        }
    }
}

struct LoadOwner<'a> {
    cache: &'a NativeModelCatalogCache,
    previous: Option<NativeModelCatalogCacheSnapshot>,
}

impl Drop for LoadOwner<'_> {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            self.cache.lock().snapshot = previous;
            self.cache.wake_waiters();
        }
    }
}

struct Waiter<'a> {
    cache: &'a NativeModelCatalogCache,
    slot: Option<usize>,
}

impl Drop for Waiter<'_> {
    fn drop(&mut self) {
        if let Some(slot) = self.slot {
            let removed = self.cache.lock().waiters[slot].take();
            drop(removed);
        }
    }
}
