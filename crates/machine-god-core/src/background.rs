//! Provider-neutral admission for durably recorded background processes.

use crate::{BoxFuture, CancellationToken};
use core::fmt;
use core::future::Future;
use core::num::NonZeroU32;
use std::sync::Arc;
use std::task::Poll;

/// Maximum UTF-8 bytes in one background shell command.
pub const MAX_BACKGROUND_COMMAND_BYTES: usize = 32 * 1024;
/// Maximum UTF-8 bytes in one canonical background working directory.
pub const MAX_BACKGROUND_CWD_BYTES: usize = 4 * 1024;

/// Stable category for a background admission failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BackgroundStartErrorKind {
    /// The request did not satisfy its fixed structural bounds.
    InvalidRequest,
    /// The bounded native retention set had no immediately available slot.
    Capacity,
    /// The explicitly injected clock could not produce a timestamp.
    Clock,
    /// Durable reservation or record publication failed.
    Persistence,
    /// Process preparation or release failed.
    Process,
    /// Cancellation won before the prepared process was released.
    Cancelled,
}

/// Fixed, data-free background admission failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackgroundStartError {
    kind: BackgroundStartErrorKind,
}

impl BackgroundStartError {
    /// Constructs one fixed failure category.
    #[must_use]
    pub const fn new(kind: BackgroundStartErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the stable machine-readable category.
    #[must_use]
    pub const fn kind(self) -> BackgroundStartErrorKind {
        self.kind
    }
}

impl fmt::Display for BackgroundStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            BackgroundStartErrorKind::InvalidRequest => "background request is invalid",
            BackgroundStartErrorKind::Capacity => "background capacity is unavailable",
            BackgroundStartErrorKind::Clock => "background clock is unavailable",
            BackgroundStartErrorKind::Persistence => "background persistence is unavailable",
            BackgroundStartErrorKind::Process => "background process could not start",
            BackgroundStartErrorKind::Cancelled => "background start was cancelled",
        })
    }
}

impl std::error::Error for BackgroundStartError {}

/// One bounded, noninteractive background start request.
///
/// The working directory is an absolute canonical Unicode path in the native
/// persisted-record format. Core performs only lexical validation; the native
/// adapter remains responsible for descriptor-bound canonicalization and for
/// retaining the verified directory through process preparation.
#[derive(Clone, Eq, PartialEq)]
pub struct BackgroundStartRequest {
    command: Box<str>,
    cwd: Box<str>,
}

impl BackgroundStartRequest {
    /// Validates and owns one request without exercising external authority.
    ///
    /// # Errors
    ///
    /// Returns a fixed invalid-request failure for an empty, NUL-containing,
    /// oversized command or for a noncanonical persisted working directory.
    pub fn new(
        command: impl Into<String>,
        cwd: impl Into<String>,
    ) -> Result<Self, BackgroundStartError> {
        let command = command.into();
        let cwd = cwd.into();
        if command.is_empty()
            || command.len() > MAX_BACKGROUND_COMMAND_BYTES
            || command.contains('\0')
            || !canonical_persisted_path(&cwd)
        {
            return Err(BackgroundStartError::new(
                BackgroundStartErrorKind::InvalidRequest,
            ));
        }
        Ok(Self {
            command: command.into_boxed_str(),
            cwd: cwd.into_boxed_str(),
        })
    }

    /// Returns the exact bounded shell command.
    #[must_use]
    pub const fn command(&self) -> &str {
        &self.command
    }

    /// Returns the canonical persisted working directory.
    #[must_use]
    pub const fn cwd(&self) -> &str {
        &self.cwd
    }
}

impl fmt::Debug for BackgroundStartRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackgroundStartRequest")
            .finish_non_exhaustive()
    }
}

fn canonical_persisted_path(path: &str) -> bool {
    if path.is_empty()
        || path.len() > MAX_BACKGROUND_CWD_BYTES
        || path.contains('\0')
        || !path.starts_with('/')
    {
        return false;
    }
    path == "/"
        || (!path.ends_with('/')
            && path
                .split('/')
                .skip(1)
                .all(|component| !component.is_empty() && component != "." && component != ".."))
}

/// Explicit provider-neutral clock used by admission.
///
/// Implementations must complete in bounded nonblocking work and return no
/// secret-bearing diagnostics. Core calls this interface only after the start
/// future is polled and a retention permit has been admitted.
pub trait BackgroundClock: Send + Sync + 'static {
    /// Returns milliseconds since the host's selected epoch.
    ///
    /// # Errors
    ///
    /// Returns a fixed clock failure when no valid timestamp is available.
    fn now_millis(&self) -> Result<u64, BackgroundStartError>;
}

/// Complete initially published state for one prepared background process.
#[derive(Clone, Eq, PartialEq)]
pub struct BackgroundRunningRecord {
    id: u64,
    started_at_ms: u64,
    command: Box<str>,
    cwd: Box<str>,
    pid: Option<NonZeroU32>,
}

impl BackgroundRunningRecord {
    fn new(
        id: u64,
        started_at_ms: u64,
        request: &BackgroundStartRequest,
        pid: Option<NonZeroU32>,
    ) -> Self {
        Self {
            id,
            started_at_ms,
            command: request.command.clone(),
            cwd: request.cwd.clone(),
            pid,
        }
    }

    /// Returns the durably reserved numeric display identity.
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// Returns the initial clock timestamp used for both initial timestamps.
    #[must_use]
    pub const fn started_at_ms(&self) -> u64 {
        self.started_at_ms
    }

    /// Returns the exact bounded command.
    #[must_use]
    pub const fn command(&self) -> &str {
        &self.command
    }

    /// Returns the canonical persisted working directory.
    #[must_use]
    pub const fn cwd(&self) -> &str {
        &self.cwd
    }

    /// Returns the prepared process ID for display and diagnostics only.
    ///
    /// This value is never a process-control capability, liveness proof, or
    /// substitute for the retained owned-process handle.
    #[must_use]
    pub const fn pid(&self) -> Option<NonZeroU32> {
        self.pid
    }
}

impl fmt::Debug for BackgroundRunningRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackgroundRunningRecord")
            .finish_non_exhaustive()
    }
}

/// Terminal process observation supplied by an owned native process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BackgroundProcessOutcome {
    /// The process exited normally with the given platform exit code.
    Exited(i32),
    /// The process was explicitly stopped; a platform exit code may exist.
    Stopped(Option<i32>),
    /// Completion could not be observed reliably after ownership was retained.
    Dead,
}

/// One terminal replacement for an initially published running record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackgroundCompletionRecord {
    updated_at_ms: u64,
    outcome: BackgroundProcessOutcome,
}

impl BackgroundCompletionRecord {
    /// Returns the terminal observation timestamp.
    #[must_use]
    pub const fn updated_at_ms(self) -> u64 {
        self.updated_at_ms
    }

    /// Returns the owned-process terminal observation.
    #[must_use]
    pub const fn outcome(self) -> BackgroundProcessOutcome {
        self.outcome
    }
}

impl BackgroundRunningRecord {
    /// Builds a timestamp-valid terminal replacement for this record.
    ///
    /// # Errors
    ///
    /// Returns a fixed clock failure if the completion timestamp precedes the
    /// initial timestamp.
    pub const fn completion(
        &self,
        updated_at_ms: u64,
        outcome: BackgroundProcessOutcome,
    ) -> Result<BackgroundCompletionRecord, BackgroundStartError> {
        if updated_at_ms < self.started_at_ms {
            return Err(BackgroundStartError::new(BackgroundStartErrorKind::Clock));
        }
        Ok(BackgroundCompletionRecord {
            updated_at_ms,
            outcome,
        })
    }
}

/// Durable per-record reservation held from ID allocation through completion.
///
/// `id` is a display and persistence identity only. Implementations must hold
/// any native record lease for the entire object lifetime. `publish_initial`
/// atomically publishes a complete private `running` record or publishes
/// nothing; returning success is the commit barrier before process release.
/// Constructing either returned future is inert.
pub trait BackgroundRecordLease: Send + Sync + 'static {
    /// Returns the durably reserved nonzero numeric ID.
    fn id(&self) -> u64;

    /// Publishes the complete initial running record atomically.
    fn publish_initial<'a>(
        &'a self,
        record: &'a BackgroundRunningRecord,
    ) -> BoxFuture<'a, Result<(), BackgroundStartError>>;

    /// Atomically replaces the initial record with one terminal observation.
    ///
    /// Native retainers call this while continuing to own both the lease and
    /// process. The replacement must preserve the initial identity, command,
    /// cwd, start timestamp, and display PID. Future construction is inert.
    fn publish_completion<'a>(
        &'a self,
        initial: &'a BackgroundRunningRecord,
        completion: &'a BackgroundCompletionRecord,
    ) -> BoxFuture<'a, Result<(), BackgroundStartError>>;
}

/// Explicit durable reservation authority.
///
/// Successful reservation must durably allocate a unique ID before returning.
/// Gaps are permitted after a later admission failure. Future construction is
/// inert, and dropping the future or returned lease must release all temporary
/// reservation resources without executing a command.
pub trait BackgroundStore: Send + Sync + 'static {
    /// Reserves one durable background record identity.
    fn reserve(
        &self,
    ) -> BoxFuture<'_, Result<Box<dyn BackgroundRecordLease>, BackgroundStartError>>;
}

/// A process held behind a native execution barrier.
///
/// Dropping this value before successful release must synchronously close the
/// barrier, terminate and reap every prepared child resource, and guarantee
/// that the requested command did not execute. `release` is the sole operation
/// that may open the barrier. On a release error the implementation retains the
/// same no-command-executed guarantee and completes cleanup before returning.
pub trait PreparedBackgroundProcess: Send + 'static {
    /// Returns a display-only process ID, when the platform exposes one.
    fn pid(&self) -> Option<NonZeroU32>;

    /// Opens the execution barrier and transfers the owned process handle.
    ///
    /// # Errors
    ///
    /// Returns a fixed process failure only after guaranteeing the command did
    /// not execute and every prepared native resource was cleaned.
    fn release(self: Box<Self>) -> Result<Box<dyn OwnedBackgroundProcess>, BackgroundStartError>;
}

/// A released native process whose ownership is sufficient to observe its
/// completion without PID-based lookup.
pub trait OwnedBackgroundProcess: Send + 'static {
    /// Returns a display-only process ID, when the platform exposes one.
    fn pid(&self) -> Option<NonZeroU32>;

    /// Consumes ownership and waits for one terminal observation.
    ///
    /// Future construction must be inert. Cancelling `stop` requests owned
    /// process-tree termination and reap and must resolve as `Stopped` once
    /// cleanup completes. Dropping the wait future must perform the same
    /// cleanup; it must never detach, leak, or transfer the owned process.
    fn wait(
        self: Box<Self>,
        stop: CancellationToken,
    ) -> BoxFuture<'static, Result<BackgroundProcessOutcome, BackgroundStartError>>;
}

/// Explicit process preparation authority.
///
/// `prepare` may create native process resources, but the command must remain
/// behind an execution barrier. The returned future owns all in-progress
/// preparation; dropping it must clean every resource and guarantee that the
/// command did not execute. The native adapter may retain descriptor-bound cwd
/// and environment authority inside that future and the prepared process.
pub trait BackgroundProcessSpawner: Send + Sync + 'static {
    /// Prepares one barrier-held process.
    fn prepare<'a>(
        &'a self,
        request: &'a BackgroundStartRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<Box<dyn PreparedBackgroundProcess>, BackgroundStartError>>;
}

/// One fail-fast native retention reservation.
///
/// Dropping an unused permit releases its capacity. `retain` is deliberately
/// infallible: after release there must be exactly one owner responsible for
/// waiting, completion publication, and final cleanup. The record lease must
/// remain owned until that responsibility is complete.
pub trait BackgroundRetentionPermit: Send + 'static {
    /// Irrevocably transfers the complete released operation to the retainer.
    fn retain(
        self: Box<Self>,
        lease: Box<dyn BackgroundRecordLease>,
        record: BackgroundRunningRecord,
        process: Box<dyn OwnedBackgroundProcess>,
    );
}

/// Bounded native background-process retainer.
pub trait BackgroundProcessRetainer: Send + Sync + 'static {
    /// Acquires capacity synchronously without waiting or external I/O.
    ///
    /// Implementations must use bounded nonblocking work and return `Capacity`
    /// immediately when no slot is available.
    ///
    /// # Errors
    ///
    /// Returns a fixed capacity failure when no slot is immediately available.
    fn try_admit(&self) -> Result<Box<dyn BackgroundRetentionPermit>, BackgroundStartError>;
}

/// Display identity returned after durable publication and ownership transfer.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct BackgroundHandle {
    id: u64,
    pid: Option<NonZeroU32>,
}

impl fmt::Debug for BackgroundHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackgroundHandle")
            .finish_non_exhaustive()
    }
}

impl BackgroundHandle {
    /// Returns the durable numeric display identity.
    #[must_use]
    pub const fn id(self) -> u64 {
        self.id
    }

    /// Returns the display-only process ID, when available.
    #[must_use]
    pub const fn pid(self) -> Option<NonZeroU32> {
        self.pid
    }
}

/// Provider-neutral coordinator for one bounded background start.
pub struct BackgroundSupervisor {
    clock: Arc<dyn BackgroundClock>,
    store: Arc<dyn BackgroundStore>,
    spawner: Arc<dyn BackgroundProcessSpawner>,
    retainer: Arc<dyn BackgroundProcessRetainer>,
}

impl BackgroundSupervisor {
    /// Constructs an inert coordinator from explicitly injected authorities.
    #[must_use]
    pub fn new(
        clock: Arc<dyn BackgroundClock>,
        store: Arc<dyn BackgroundStore>,
        spawner: Arc<dyn BackgroundProcessSpawner>,
        retainer: Arc<dyn BackgroundProcessRetainer>,
    ) -> Self {
        Self {
            clock,
            store,
            spawner,
            retainer,
        }
    }

    /// Starts one bounded operation after durable initial publication.
    ///
    /// Constructing this future is inert. On first poll, admission is
    /// fail-fast, then core orders durable ID reservation, explicit clock read,
    /// process preparation, complete initial publication, barrier release, and
    /// infallible ownership transfer. Cancellation can win only before release;
    /// it then drops and cleans the prepared process. Once release begins,
    /// cancellation cannot revoke ownership or change the returned result.
    #[must_use]
    pub fn start(
        &self,
        request: BackgroundStartRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<BackgroundHandle, BackgroundStartError>> {
        let clock = Arc::clone(&self.clock);
        let store = Arc::clone(&self.store);
        let spawner = Arc::clone(&self.spawner);
        let retainer = Arc::clone(&self.retainer);
        Box::pin(async move {
            start_polled(clock, store, spawner, retainer, request, cancellation).await
        })
    }
}

impl fmt::Debug for BackgroundSupervisor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackgroundSupervisor")
            .finish_non_exhaustive()
    }
}

async fn start_polled(
    clock: Arc<dyn BackgroundClock>,
    store: Arc<dyn BackgroundStore>,
    spawner: Arc<dyn BackgroundProcessSpawner>,
    retainer: Arc<dyn BackgroundProcessRetainer>,
    request: BackgroundStartRequest,
    cancellation: CancellationToken,
) -> Result<BackgroundHandle, BackgroundStartError> {
    check_cancellation(&cancellation)?;
    let permit = retainer.try_admit()?;
    check_cancellation(&cancellation)?;

    let lease = await_or_cancel(store.reserve(), &cancellation).await?;
    let id = lease.id();
    if id == 0 {
        return Err(BackgroundStartError::new(
            BackgroundStartErrorKind::Persistence,
        ));
    }
    let started_at_ms = clock.now_millis()?;
    check_cancellation(&cancellation)?;

    let prepared = await_or_cancel(
        spawner.prepare(&request, cancellation.clone()),
        &cancellation,
    )
    .await?;
    check_cancellation(&cancellation)?;

    let pid = prepared.pid();
    let record = BackgroundRunningRecord::new(id, started_at_ms, &request, pid);
    await_commit_or_cancel(lease.publish_initial(&record), &cancellation).await?;
    if cancellation.is_cancelled() {
        drop(prepared);
        if let Ok(completion) =
            record.completion(started_at_ms, BackgroundProcessOutcome::Stopped(None))
        {
            let _ = lease.publish_completion(&record, &completion).await;
        }
        return Err(BackgroundStartError::new(
            BackgroundStartErrorKind::Cancelled,
        ));
    }

    // Release is the irreversible commit point. The cancellation observation
    // above is deliberately the last one; cancellation cannot revoke ownership
    // after this call begins.
    let process = match prepared.release() {
        Ok(process) => process,
        Err(error) => {
            if let Ok(completion) = record.completion(started_at_ms, BackgroundProcessOutcome::Dead)
            {
                let _ = lease.publish_completion(&record, &completion).await;
            }
            return Err(error);
        }
    };
    debug_assert_eq!(process.pid(), pid);
    let handle = BackgroundHandle {
        id: record.id(),
        pid,
    };
    permit.retain(lease, record, process);
    Ok(handle)
}

fn check_cancellation(cancellation: &CancellationToken) -> Result<(), BackgroundStartError> {
    if cancellation.is_cancelled() {
        Err(BackgroundStartError::new(
            BackgroundStartErrorKind::Cancelled,
        ))
    } else {
        Ok(())
    }
}

async fn await_or_cancel<T>(
    mut operation: BoxFuture<'_, Result<T, BackgroundStartError>>,
    cancellation: &CancellationToken,
) -> Result<T, BackgroundStartError> {
    let mut cancellation_wait = Box::pin(cancellation.cancelled());
    std::future::poll_fn(|context| {
        if cancellation_wait.as_mut().poll(context).is_ready() {
            return Poll::Ready(Err(BackgroundStartError::new(
                BackgroundStartErrorKind::Cancelled,
            )));
        }
        let result = operation.as_mut().poll(context);
        if cancellation.is_cancelled() {
            Poll::Ready(Err(BackgroundStartError::new(
                BackgroundStartErrorKind::Cancelled,
            )))
        } else {
            result
        }
    })
    .await
}

async fn await_commit_or_cancel<T>(
    mut operation: BoxFuture<'_, Result<T, BackgroundStartError>>,
    cancellation: &CancellationToken,
) -> Result<T, BackgroundStartError> {
    let mut cancellation_wait = Box::pin(cancellation.cancelled());
    std::future::poll_fn(|context| {
        if cancellation_wait.as_mut().poll(context).is_ready() {
            Poll::Ready(Err(BackgroundStartError::new(
                BackgroundStartErrorKind::Cancelled,
            )))
        } else {
            operation.as_mut().poll(context)
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{
        BackgroundClock, BackgroundCompletionRecord, BackgroundHandle, BackgroundProcessOutcome,
        BackgroundProcessRetainer, BackgroundProcessSpawner, BackgroundRecordLease,
        BackgroundRetentionPermit, BackgroundRunningRecord, BackgroundStartError,
        BackgroundStartErrorKind, BackgroundStartRequest, BackgroundStore, BackgroundSupervisor,
        MAX_BACKGROUND_COMMAND_BYTES, MAX_BACKGROUND_CWD_BYTES, OwnedBackgroundProcess,
        PreparedBackgroundProcess,
    };
    use crate::{BoxFuture, CancellationToken};
    use core::future::Future;
    use core::num::NonZeroU32;
    use core::pin::Pin;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use core::task::{Context, Poll};
    use futures_executor::block_on;
    use futures_util::task::noop_waker_ref;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Copy, Debug, Default)]
    #[allow(clippy::struct_excessive_bools)]
    struct Failures {
        capacity: bool,
        reserve: bool,
        clock: bool,
        prepare: bool,
        publish: bool,
        completion: bool,
        release: bool,
    }

    #[derive(Debug, Default)]
    struct Observations {
        events: Mutex<Vec<&'static str>>,
        aborted: AtomicUsize,
        executed: AtomicUsize,
        retained: AtomicUsize,
        completion_publications: AtomicUsize,
    }

    impl Observations {
        fn event(&self, event: &'static str) {
            self.events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(event);
        }

        fn events(&self) -> Vec<&'static str> {
            self.events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    #[derive(Debug)]
    struct FakeClock {
        observations: Arc<Observations>,
        fail: bool,
    }

    impl BackgroundClock for FakeClock {
        fn now_millis(&self) -> Result<u64, BackgroundStartError> {
            self.observations.event("clock");
            if self.fail {
                Err(error(BackgroundStartErrorKind::Clock))
            } else {
                Ok(1_234)
            }
        }
    }

    #[derive(Debug)]
    struct FakeStore {
        observations: Arc<Observations>,
        reserve_fail: bool,
        publish_fail: bool,
        completion_fail: bool,
        cancel_during_publish: Option<CancellationToken>,
    }

    impl BackgroundStore for FakeStore {
        fn reserve(
            &self,
        ) -> BoxFuture<'_, Result<Box<dyn BackgroundRecordLease>, BackgroundStartError>> {
            Box::pin(async move {
                self.observations.event("reserve");
                if self.reserve_fail {
                    Err(error(BackgroundStartErrorKind::Persistence))
                } else {
                    Ok(Box::new(FakeLease {
                        observations: Arc::clone(&self.observations),
                        publish_fail: self.publish_fail,
                        completion_fail: self.completion_fail,
                        cancel_during_publish: self.cancel_during_publish.clone(),
                    }) as Box<dyn BackgroundRecordLease>)
                }
            })
        }
    }

    #[derive(Debug)]
    struct FakeLease {
        observations: Arc<Observations>,
        publish_fail: bool,
        completion_fail: bool,
        cancel_during_publish: Option<CancellationToken>,
    }

    impl BackgroundRecordLease for FakeLease {
        fn id(&self) -> u64 {
            7
        }

        fn publish_initial<'a>(
            &'a self,
            record: &'a BackgroundRunningRecord,
        ) -> BoxFuture<'a, Result<(), BackgroundStartError>> {
            Box::pin(async move {
                self.observations.event("publish");
                assert_eq!(record.id(), 7);
                assert_eq!(record.started_at_ms(), 1_234);
                assert_eq!(record.command(), "echo ready");
                assert_eq!(record.cwd(), "/workspace");
                if let Some(cancellation) = &self.cancel_during_publish {
                    cancellation.cancel();
                }
                if self.publish_fail {
                    Err(error(BackgroundStartErrorKind::Persistence))
                } else {
                    Ok(())
                }
            })
        }

        fn publish_completion<'a>(
            &'a self,
            initial: &'a BackgroundRunningRecord,
            completion: &'a BackgroundCompletionRecord,
        ) -> BoxFuture<'a, Result<(), BackgroundStartError>> {
            Box::pin(async move {
                self.observations.event("complete");
                self.observations
                    .completion_publications
                    .fetch_add(1, Ordering::Relaxed);
                assert_eq!(initial.id(), 7);
                assert_eq!(completion.updated_at_ms(), 1_234);
                assert!(matches!(
                    completion.outcome(),
                    BackgroundProcessOutcome::Dead | BackgroundProcessOutcome::Stopped(None)
                ));
                if self.completion_fail {
                    Err(error(BackgroundStartErrorKind::Persistence))
                } else {
                    Ok(())
                }
            })
        }
    }

    #[derive(Debug)]
    struct FakeSpawner {
        observations: Arc<Observations>,
        prepare_fail: bool,
        release_fail: bool,
        cancel_during_prepare: Option<CancellationToken>,
        stay_pending: bool,
        pending_dropped: Arc<AtomicUsize>,
    }

    impl BackgroundProcessSpawner for FakeSpawner {
        fn prepare<'a>(
            &'a self,
            _request: &'a BackgroundStartRequest,
            _cancellation: CancellationToken,
        ) -> BoxFuture<'a, Result<Box<dyn PreparedBackgroundProcess>, BackgroundStartError>>
        {
            Box::pin(async move {
                self.observations.event("prepare");
                let _drop_probe = DropCounter(Arc::clone(&self.pending_dropped));
                if self.stay_pending {
                    std::future::pending::<()>().await;
                }
                if self.prepare_fail {
                    return Err(error(BackgroundStartErrorKind::Process));
                }
                let prepared: Box<dyn PreparedBackgroundProcess> = Box::new(FakePrepared {
                    observations: Arc::clone(&self.observations),
                    released: false,
                    release_fail: self.release_fail,
                });
                if let Some(cancellation) = &self.cancel_during_prepare {
                    cancellation.cancel();
                }
                Ok(prepared)
            })
        }
    }

    #[derive(Debug)]
    struct DropCounter(Arc<AtomicUsize>);

    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[derive(Debug)]
    struct FakePrepared {
        observations: Arc<Observations>,
        released: bool,
        release_fail: bool,
    }

    impl Drop for FakePrepared {
        fn drop(&mut self) {
            if !self.released {
                self.observations.aborted.fetch_add(1, Ordering::Relaxed);
                self.observations.event("abort");
            }
        }
    }

    impl PreparedBackgroundProcess for FakePrepared {
        fn pid(&self) -> Option<NonZeroU32> {
            NonZeroU32::new(42)
        }

        fn release(
            mut self: Box<Self>,
        ) -> Result<Box<dyn OwnedBackgroundProcess>, BackgroundStartError> {
            self.observations.event("release");
            if self.release_fail {
                self.released = true;
                return Err(error(BackgroundStartErrorKind::Process));
            }
            self.observations.executed.fetch_add(1, Ordering::Relaxed);
            self.released = true;
            Ok(Box::new(FakeOwned))
        }
    }

    #[derive(Debug)]
    struct FakeOwned;

    impl OwnedBackgroundProcess for FakeOwned {
        fn pid(&self) -> Option<NonZeroU32> {
            NonZeroU32::new(42)
        }

        fn wait(
            self: Box<Self>,
            _stop: CancellationToken,
        ) -> BoxFuture<'static, Result<BackgroundProcessOutcome, BackgroundStartError>> {
            Box::pin(async { Ok(BackgroundProcessOutcome::Exited(0)) })
        }
    }

    #[derive(Debug)]
    struct FakeRetainer {
        observations: Arc<Observations>,
        capacity_fail: bool,
        cancel_during_retain: Option<CancellationToken>,
    }

    impl BackgroundProcessRetainer for FakeRetainer {
        fn try_admit(&self) -> Result<Box<dyn BackgroundRetentionPermit>, BackgroundStartError> {
            self.observations.event("admit");
            if self.capacity_fail {
                Err(error(BackgroundStartErrorKind::Capacity))
            } else {
                Ok(Box::new(FakePermit {
                    observations: Arc::clone(&self.observations),
                    cancel_during_retain: self.cancel_during_retain.clone(),
                }))
            }
        }
    }

    #[derive(Debug)]
    struct FakePermit {
        observations: Arc<Observations>,
        cancel_during_retain: Option<CancellationToken>,
    }

    impl BackgroundRetentionPermit for FakePermit {
        fn retain(
            self: Box<Self>,
            _lease: Box<dyn BackgroundRecordLease>,
            record: BackgroundRunningRecord,
            process: Box<dyn OwnedBackgroundProcess>,
        ) {
            self.observations.event("retain");
            assert_eq!(record.pid(), process.pid());
            self.observations.retained.fetch_add(1, Ordering::Relaxed);
            if let Some(cancellation) = &self.cancel_during_retain {
                cancellation.cancel();
            }
        }
    }

    fn error(kind: BackgroundStartErrorKind) -> BackgroundStartError {
        BackgroundStartError::new(kind)
    }

    fn request() -> BackgroundStartRequest {
        BackgroundStartRequest::new("echo ready", "/workspace").unwrap()
    }

    fn fixture(
        failures: Failures,
        cancellation: Option<CancellationToken>,
        cancel_stage: Option<&str>,
        stay_pending: bool,
    ) -> (BackgroundSupervisor, Arc<Observations>, Arc<AtomicUsize>) {
        let observations = Arc::new(Observations::default());
        let pending_dropped = Arc::new(AtomicUsize::new(0));
        let supervisor = BackgroundSupervisor::new(
            Arc::new(FakeClock {
                observations: Arc::clone(&observations),
                fail: failures.clock,
            }),
            Arc::new(FakeStore {
                observations: Arc::clone(&observations),
                reserve_fail: failures.reserve,
                publish_fail: failures.publish,
                completion_fail: failures.completion,
                cancel_during_publish: (cancel_stage == Some("publish"))
                    .then_some(cancellation.clone())
                    .flatten(),
            }),
            Arc::new(FakeSpawner {
                observations: Arc::clone(&observations),
                prepare_fail: failures.prepare,
                release_fail: failures.release,
                cancel_during_prepare: (cancel_stage == Some("prepare"))
                    .then_some(cancellation.clone())
                    .flatten(),
                stay_pending,
                pending_dropped: Arc::clone(&pending_dropped),
            }),
            Arc::new(FakeRetainer {
                observations: Arc::clone(&observations),
                capacity_fail: failures.capacity,
                cancel_during_retain: (cancel_stage == Some("retain"))
                    .then_some(cancellation)
                    .flatten(),
            }),
        );
        (supervisor, observations, pending_dropped)
    }

    #[test]
    fn request_enforces_exact_bounds_and_canonical_paths() {
        assert!(
            BackgroundStartRequest::new(
                "x".repeat(MAX_BACKGROUND_COMMAND_BYTES),
                format!("/{}", "w".repeat(MAX_BACKGROUND_CWD_BYTES - 1)),
            )
            .is_ok()
        );
        for (command, cwd) in [
            (String::new(), "/workspace".to_owned()),
            ("x\0y".to_owned(), "/workspace".to_owned()),
            (
                "x".repeat(MAX_BACKGROUND_COMMAND_BYTES + 1),
                "/workspace".to_owned(),
            ),
            ("x".to_owned(), String::new()),
            ("x".to_owned(), "workspace".to_owned()),
            ("x".to_owned(), "/workspace/".to_owned()),
            ("x".to_owned(), "//workspace".to_owned()),
            ("x".to_owned(), "/workspace/./child".to_owned()),
            ("x".to_owned(), "/workspace/../child".to_owned()),
            (
                "x".to_owned(),
                format!("/{}", "w".repeat(MAX_BACKGROUND_CWD_BYTES)),
            ),
        ] {
            assert_eq!(
                BackgroundStartRequest::new(command, cwd)
                    .unwrap_err()
                    .kind(),
                BackgroundStartErrorKind::InvalidRequest
            );
        }
        assert!(BackgroundStartRequest::new("x", "/").is_ok());
    }

    #[test]
    fn request_record_and_errors_do_not_debug_secrets() {
        let request = BackgroundStartRequest::new("PRIVATE_COMMAND", "/PRIVATE_CWD").unwrap();
        assert!(!format!("{request:?}").contains("PRIVATE"));
        let record = BackgroundRunningRecord::new(
            9_876_543_210,
            8_765_432_109,
            &request,
            NonZeroU32::new(4_000_000_007),
        );
        assert_eq!(format!("{record:?}"), "BackgroundRunningRecord { .. }");
        let handle = BackgroundHandle {
            id: 7_654_321_098,
            pid: NonZeroU32::new(4_000_000_009),
        };
        assert_eq!(format!("{handle:?}"), "BackgroundHandle { .. }");
        for kind in [
            BackgroundStartErrorKind::InvalidRequest,
            BackgroundStartErrorKind::Capacity,
            BackgroundStartErrorKind::Clock,
            BackgroundStartErrorKind::Persistence,
            BackgroundStartErrorKind::Process,
            BackgroundStartErrorKind::Cancelled,
        ] {
            let diagnostic = error(kind).to_string();
            assert!(!diagnostic.contains("PRIVATE"));
        }
    }

    #[test]
    fn constructors_and_unpolled_start_are_inert() {
        let (supervisor, observations, _) = fixture(Failures::default(), None, None, false);
        let future = supervisor.start(request(), CancellationToken::new());
        assert!(observations.events().is_empty());
        drop(future);
        assert!(observations.events().is_empty());
    }

    #[test]
    fn success_preserves_the_complete_admission_order() {
        let (supervisor, observations, _) = fixture(Failures::default(), None, None, false);
        let handle = block_on(supervisor.start(request(), CancellationToken::new())).unwrap();
        assert_eq!(handle.id(), 7);
        assert_eq!(handle.pid(), NonZeroU32::new(42));
        assert_eq!(
            observations.events(),
            [
                "admit", "reserve", "clock", "prepare", "publish", "release", "retain"
            ]
        );
        assert_eq!(observations.executed.load(Ordering::Relaxed), 1);
        assert_eq!(observations.retained.load(Ordering::Relaxed), 1);
        assert_eq!(observations.aborted.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn fail_fast_capacity_precedes_every_external_authority() {
        let failures = Failures {
            capacity: true,
            ..Failures::default()
        };
        let (supervisor, observations, _) = fixture(failures, None, None, false);
        let result = block_on(supervisor.start(request(), CancellationToken::new()));
        assert_eq!(
            result.unwrap_err().kind(),
            BackgroundStartErrorKind::Capacity
        );
        assert_eq!(observations.events(), ["admit"]);
    }

    #[test]
    fn each_prepublication_failure_prevents_command_execution() {
        for (failures, expected_events) in [
            (
                Failures {
                    reserve: true,
                    ..Failures::default()
                },
                vec!["admit", "reserve"],
            ),
            (
                Failures {
                    clock: true,
                    ..Failures::default()
                },
                vec!["admit", "reserve", "clock"],
            ),
            (
                Failures {
                    prepare: true,
                    ..Failures::default()
                },
                vec!["admit", "reserve", "clock", "prepare"],
            ),
            (
                Failures {
                    publish: true,
                    ..Failures::default()
                },
                vec!["admit", "reserve", "clock", "prepare", "publish", "abort"],
            ),
        ] {
            let (supervisor, observations, _) = fixture(failures, None, None, false);
            assert!(block_on(supervisor.start(request(), CancellationToken::new())).is_err());
            assert_eq!(observations.events(), expected_events);
            assert_eq!(observations.executed.load(Ordering::Relaxed), 0);
            assert_eq!(observations.retained.load(Ordering::Relaxed), 0);
        }
    }

    #[test]
    fn release_failure_is_reaped_and_replaces_the_running_record() {
        let failures = Failures {
            release: true,
            ..Failures::default()
        };
        let (supervisor, observations, _) = fixture(failures, None, None, false);
        let result = block_on(supervisor.start(request(), CancellationToken::new()));
        assert_eq!(
            result.unwrap_err().kind(),
            BackgroundStartErrorKind::Process
        );
        assert_eq!(
            observations.events(),
            [
                "admit", "reserve", "clock", "prepare", "publish", "release", "complete"
            ]
        );
        assert_eq!(observations.executed.load(Ordering::Relaxed), 0);
        assert_eq!(observations.retained.load(Ordering::Relaxed), 0);
        assert_eq!(
            observations.completion_publications.load(Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn release_failure_preserves_process_error_when_completion_publication_fails() {
        let failures = Failures {
            completion: true,
            release: true,
            ..Failures::default()
        };
        let (supervisor, observations, _) = fixture(failures, None, None, false);
        let result = block_on(supervisor.start(request(), CancellationToken::new()));
        assert_eq!(
            result.unwrap_err().kind(),
            BackgroundStartErrorKind::Process
        );
        assert_eq!(
            observations.events(),
            [
                "admit", "reserve", "clock", "prepare", "publish", "release", "complete"
            ]
        );
        assert_eq!(
            observations.completion_publications.load(Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn cancellation_before_poll_exercises_no_authority() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let (supervisor, observations, _) = fixture(Failures::default(), None, None, false);
        let result = block_on(supervisor.start(request(), cancellation));
        assert_eq!(
            result.unwrap_err().kind(),
            BackgroundStartErrorKind::Cancelled
        );
        assert!(observations.events().is_empty());
    }

    #[test]
    fn same_poll_prepare_cancellation_cleans_the_prepared_process() {
        let cancellation = CancellationToken::new();
        let (supervisor, observations, _) = fixture(
            Failures::default(),
            Some(cancellation.clone()),
            Some("prepare"),
            false,
        );
        let result = block_on(supervisor.start(request(), cancellation));
        assert_eq!(
            result.unwrap_err().kind(),
            BackgroundStartErrorKind::Cancelled
        );
        assert_eq!(
            observations.events(),
            ["admit", "reserve", "clock", "prepare", "abort"]
        );
        assert_eq!(observations.aborted.load(Ordering::Relaxed), 1);
        assert_eq!(observations.executed.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn dropping_polled_start_drops_in_progress_preparation() {
        let (supervisor, observations, pending_dropped) =
            fixture(Failures::default(), None, None, true);
        let mut future = supervisor.start(request(), CancellationToken::new());
        let mut context = Context::from_waker(noop_waker_ref());
        assert!(matches!(
            Pin::new(&mut future).poll(&mut context),
            Poll::Pending
        ));
        assert_eq!(pending_dropped.load(Ordering::Relaxed), 0);
        drop(future);
        assert_eq!(pending_dropped.load(Ordering::Relaxed), 1);
        assert_eq!(
            observations.events(),
            ["admit", "reserve", "clock", "prepare"]
        );
        assert_eq!(observations.executed.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn cancellation_during_publication_cleans_before_release() {
        let cancellation = CancellationToken::new();
        let (supervisor, observations, _) = fixture(
            Failures::default(),
            Some(cancellation.clone()),
            Some("publish"),
            false,
        );
        let result = block_on(supervisor.start(request(), cancellation.clone()));
        assert_eq!(
            result.unwrap_err().kind(),
            BackgroundStartErrorKind::Cancelled
        );
        assert!(cancellation.is_cancelled());
        assert_eq!(
            observations.events(),
            [
                "admit", "reserve", "clock", "prepare", "publish", "abort", "complete"
            ]
        );
        assert_eq!(observations.executed.load(Ordering::Relaxed), 0);
        assert_eq!(observations.retained.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn cancellation_preserves_cancelled_when_completion_publication_fails() {
        let cancellation = CancellationToken::new();
        let failures = Failures {
            completion: true,
            ..Failures::default()
        };
        let (supervisor, observations, _) =
            fixture(failures, Some(cancellation.clone()), Some("publish"), false);
        let result = block_on(supervisor.start(request(), cancellation));
        assert_eq!(
            result.unwrap_err().kind(),
            BackgroundStartErrorKind::Cancelled
        );
        assert_eq!(
            observations.events(),
            [
                "admit", "reserve", "clock", "prepare", "publish", "abort", "complete"
            ]
        );
        assert_eq!(
            observations.completion_publications.load(Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn cancellation_after_release_cannot_revoke_retained_ownership() {
        let cancellation = CancellationToken::new();
        let (supervisor, observations, _) = fixture(
            Failures::default(),
            Some(cancellation.clone()),
            Some("retain"),
            false,
        );
        let handle = block_on(supervisor.start(request(), cancellation.clone())).unwrap();
        assert!(cancellation.is_cancelled());
        assert_eq!(handle.id(), 7);
        assert_eq!(observations.retained.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn completion_rejects_a_clock_regression() {
        let record = BackgroundRunningRecord::new(1, 10, &request(), None);
        assert_eq!(
            record
                .completion(9, BackgroundProcessOutcome::Exited(0))
                .unwrap_err()
                .kind(),
            BackgroundStartErrorKind::Clock
        );
        assert_eq!(
            record
                .completion(10, BackgroundProcessOutcome::Stopped(None))
                .unwrap()
                .outcome(),
            BackgroundProcessOutcome::Stopped(None)
        );
    }
}
