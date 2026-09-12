//! Observation-only legacy URL completion routing. No consent or retry grants.

mod codec;
mod state;
#[cfg(test)]
mod tests;

use crate::mcp::runtime::NativeMcpRuntimeClock;
pub use codec::McpLegacyCompletionNotification;
use futures_util::future::{Either, select};
use machine_god_core::{BoxFuture, CancellationToken};
use std::{
    fmt,
    sync::{
        Arc, Mutex, PoisonError, Weak,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

const MAX_IDS: usize = 32;
const MAX_ID_BYTES: usize = 256;
const EARLY_TTL: Duration = Duration::from_secs(10 * 60);
const PENDING: u8 = 0;
const COMPLETE: u8 = 1;
const CANCELLED: u8 = 2;
type Result<T> = std::result::Result<T, McpCompletionError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpCompletionError {
    Invalid,
    Limit,
    Duplicate,
    Cancelled,
    Deadline,
    Unavailable,
}
impl fmt::Display for McpCompletionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MCP completion observation unavailable")
    }
}
impl std::error::Error for McpCompletionError {}

/// All limits are positive and lowerable only. Retained bytes include handles
/// and observations still owned after their registry entries are removed.
#[derive(Clone, Copy, Debug)]
pub struct McpCompletionLimits {
    pub windows: usize,
    pub waiters: usize,
    pub early_per_window: usize,
    pub candidates: usize,
    pub retained_bytes: usize,
}
impl Default for McpCompletionLimits {
    fn default() -> Self {
        Self {
            windows: 32,
            waiters: 32,
            early_per_window: 64,
            candidates: 1024,
            retained_bytes: 4 * 1024 * 1024,
        }
    }
}
impl McpCompletionLimits {
    fn validate(self) -> Result<Self> {
        let cap = Self::default();
        if [
            (self.windows, cap.windows),
            (self.waiters, cap.waiters),
            (self.early_per_window, cap.early_per_window),
            (self.candidates, cap.candidates),
            (self.retained_bytes, cap.retained_bytes),
        ]
        .iter()
        .any(|(value, maximum)| *value == 0 || value > maximum)
        {
            return Err(McpCompletionError::Limit);
        }
        Ok(self)
    }
}

/// Unique local observation identity, not an execution grant. The native owner
/// retains the same allocation only for one exact runtime/connection/client/auth
/// lifetime. Replacement requires a fresh source, never reminted numeric IDs.
#[derive(Clone, Default)]
pub struct McpCompletionSource(Arc<Source>);
#[derive(Default)]
struct Source {
    retired: AtomicBool,
    changed: CancellationToken,
}
impl McpCompletionSource {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

struct Budget {
    used: AtomicUsize,
    maximum: usize,
}
struct Charge {
    budget: Arc<Budget>,
    bytes: usize,
}
impl Budget {
    fn charge(self: &Arc<Self>, bytes: usize) -> Result<Charge> {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.maximum)
            })
            .map_err(|_| McpCompletionError::Limit)?;
        Ok(Charge {
            budget: self.clone(),
            bytes,
        })
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

struct Inner {
    state: Mutex<state::State>,
    limits: McpCompletionLimits,
    budget: Arc<Budget>,
    closed: AtomicBool,
    changed: CancellationToken,
    _state_charge: Charge,
}
/// Bounded registry; creation starts no task, transport, timer or ambient read.
pub struct McpCompletionRegistry(Arc<Inner>);
impl McpCompletionRegistry {
    /// # Errors
    /// Rejects zero/above-ceiling limits or insufficient fixed storage reservation.
    pub fn new(limits: McpCompletionLimits) -> Result<Self> {
        let limits = limits.validate()?;
        let budget = Arc::new(Budget {
            used: AtomicUsize::new(0),
            maximum: limits.retained_bytes,
        });
        let state_charge = budget.charge(state::base_charge(limits))?;
        Ok(Self(Arc::new(Inner {
            state: Mutex::new(state::State::default()),
            limits,
            budget,
            closed: AtomicBool::new(false),
            changed: CancellationToken::new(),
            _state_charge: state_charge,
        })))
    }

    /// Open before sending the originating request. Source comes from the
    /// native transport owner, never from a notification's JSON fields.
    /// # Errors
    /// Rejects closed/retired/cancelled ownership and bounded capacity exhaustion.
    pub fn open_window(
        &self,
        source: McpCompletionSource,
        cancellation: CancellationToken,
    ) -> Result<McpCompletionWindow> {
        state::open(&self.0, source, cancellation)
    }

    /// Routes to the exact local source, including every overlapping observation
    /// window. Unknown notifications without an open window are ignored.
    /// # Errors
    /// Rejects stale sources, clock overflow and exhausted retained storage.
    pub fn observe(
        &self,
        source: &McpCompletionSource,
        notification: &McpLegacyCompletionNotification,
        now: Instant,
    ) -> Result<McpCompletionRoute> {
        state::observe(&self.0, source, notification, now)
    }

    /// Permanently retires this source allocation, including in other registries.
    /// Caller must choose a fresh allocation for any later native lifetime.
    pub fn invalidate_source(&self, source: &McpCompletionSource) {
        source.0.retired.store(true, Ordering::Release);
        let removed = state::remove_source(&self.0, source);
        source.0.changed.cancel();
        drop(removed);
    }

    /// Cancels all pending observations; it does not revoke remote credentials.
    pub fn close(&self) {
        self.0.closed.store(true, Ordering::Release);
        let removed = {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            std::mem::take(&mut *state)
        };
        self.0.changed.cancel();
        drop(removed);
    }

    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.0.budget.used.load(Ordering::Acquire)
    }
}
impl Drop for McpCompletionRegistry {
    fn drop(&mut self) {
        self.close();
    }
}

/// Counted observations, not completion, permission, consent or replay receipts.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct McpCompletionRoute {
    pub completed_waiters: usize,
    pub early_windows: usize,
    pub duplicate: bool,
}

struct Window {
    id: u64,
    source: McpCompletionSource,
    cancellation: CancellationToken,
    closed: AtomicBool,
    changed: CancellationToken,
    _charge: Charge,
}
/// Non-clone owned window. Dropping it cancels its waiters and discards unknown
/// early notifications. Verified source-wide candidate tombstones remain bounded.
pub struct McpCompletionWindow {
    inner: Weak<Inner>,
    data: Arc<Window>,
}
impl McpCompletionWindow {
    /// Atomically register the complete exact elicitation-ID set before any browser
    /// consent. Already observed matching frames are promoted from this window.
    /// # Errors
    /// Rejects empty/duplicate/oversized IDs, stale ownership and exhausted limits.
    pub fn register_ids(
        &self,
        ids: &[&str],
        now: Instant,
        deadline: Instant,
    ) -> Result<McpCompletionWaiter> {
        state::register(self, ids, now, deadline)
    }
}
impl Drop for McpCompletionWindow {
    fn drop(&mut self) {
        self.data.closed.store(true, Ordering::Release);
        let removed = self
            .inner
            .upgrade()
            .map(|inner| state::remove_window(&inner, self.data.id));
        self.data.changed.cancel();
        drop(removed);
    }
}

struct Waiter {
    id: u64,
    window: Arc<Window>,
    ids: Box<[Arc<str>]>,
    deadline: Instant,
    status: AtomicU8,
    waiting: AtomicBool,
    changed: CancellationToken,
    _charge: Charge,
}

struct Waiting(Arc<Waiter>);
impl Waiting {
    fn begin(data: Arc<Waiter>) -> Result<Self> {
        data.waiting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| McpCompletionError::Limit)?;
        Ok(Self(data))
    }
}
impl Drop for Waiting {
    fn drop(&mut self) {
        self.0.waiting.store(false, Ordering::Release);
    }
}
/// Notification-only waiter; actual consent and retry custody live elsewhere.
pub struct McpCompletionWaiter {
    inner: Weak<Inner>,
    data: Arc<Waiter>,
}
impl McpCompletionWaiter {
    /// # Errors
    /// Rejects expired, closed or cancelled exact ownership.
    pub fn observation(&self, now: Instant) -> Result<Option<McpCompletionObservation>> {
        let inner = self.inner.upgrade().ok_or(McpCompletionError::Cancelled)?;
        check_waiter(&inner, &self.data, now)?;
        Ok(
            (self.data.status.load(Ordering::Acquire) == COMPLETE).then(|| {
                McpCompletionObservation {
                    inner: self.inner.clone(),
                    data: self.data.clone(),
                }
            }),
        )
    }

    /// Inert until first poll. The caller explicitly supplies all clock authority.
    /// At most one polled wait may own notification subscriptions per waiter.
    pub fn wait<'a>(
        &self,
        clock: &'a dyn NativeMcpRuntimeClock,
    ) -> BoxFuture<'a, Result<McpCompletionObservation>> {
        let inner = self.inner.clone();
        let data = self.data.clone();
        Box::pin(async move {
            let _waiting = Waiting::begin(data.clone())?;
            let inner = inner.upgrade().ok_or(McpCompletionError::Cancelled)?;
            let changed = data.changed.cancelled();
            let closed = inner.changed.cancelled();
            let source = data.window.source.0.changed.cancelled();
            let window = data.window.changed.cancelled();
            let operation = data.window.cancellation.cancelled();
            check_waiter(&inner, &data, clock.now())?;
            if data.status.load(Ordering::Acquire) != COMPLETE {
                let cancellation = Box::pin(async {
                    select(
                        Box::pin(async { select(closed, source).await }),
                        Box::pin(async { select(window, operation).await }),
                    )
                    .await;
                });
                let notification = Box::pin(async { select(changed, cancellation).await });
                if matches!(
                    select(notification, clock.sleep_until(data.deadline)).await,
                    Either::Right(_)
                ) {
                    return Err(McpCompletionError::Deadline);
                }
            }
            check_waiter(&inner, &data, clock.now())?;
            if data.status.load(Ordering::Acquire) != COMPLETE {
                return Err(McpCompletionError::Cancelled);
            }
            Ok(McpCompletionObservation {
                inner: Arc::downgrade(&inner),
                data,
            })
        })
    }
}
impl Drop for McpCompletionWaiter {
    fn drop(&mut self) {
        self.data.status.store(CANCELLED, Ordering::Release);
        let removed = self
            .inner
            .upgrade()
            .map(|inner| state::remove_waiter(&inner, self.data.id));
        self.data.changed.cancel();
        drop(removed);
    }
}

/// Immutable notification observation bound to one exact native window/key set.
/// It is deliberately neither a retry permit nor evidence of human consent.
pub struct McpCompletionObservation {
    inner: Weak<Inner>,
    data: Arc<Waiter>,
}
impl McpCompletionObservation {
    #[must_use]
    pub fn belongs_to(&self, waiter: &McpCompletionWaiter) -> bool {
        Arc::ptr_eq(&self.data, &waiter.data)
    }

    /// # Errors
    /// Rejects stale window/source/registry ownership or the exact human deadline.
    pub fn revalidate(&self, now: Instant) -> Result<()> {
        let inner = self.inner.upgrade().ok_or(McpCompletionError::Cancelled)?;
        check_waiter(&inner, &self.data, now)
    }
}

fn check_window(inner: &Inner, window: &Window) -> Result<()> {
    if inner.closed.load(Ordering::Acquire)
        || window.closed.load(Ordering::Acquire)
        || window.source.0.retired.load(Ordering::Acquire)
        || window.cancellation.is_cancelled()
    {
        Err(McpCompletionError::Cancelled)
    } else {
        Ok(())
    }
}
fn check_waiter(inner: &Inner, waiter: &Waiter, now: Instant) -> Result<()> {
    check_window(inner, &waiter.window)?;
    if waiter.status.load(Ordering::Acquire) == CANCELLED {
        return Err(McpCompletionError::Cancelled);
    }
    if now >= waiter.deadline {
        return Err(McpCompletionError::Deadline);
    }
    Ok(())
}

macro_rules! redacted {
    ($($name:ty),+ $(,)?) => {$ (
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($name), " { <redacted> }"))
            }
        }
    )+};
}
redacted!(
    McpCompletionSource,
    McpCompletionRegistry,
    McpCompletionWindow,
    McpCompletionWaiter,
    McpCompletionObservation
);
