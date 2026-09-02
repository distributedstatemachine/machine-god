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
    /// Process preparation, release, or cleanup failed.
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
#[derive(Clone, Copy, Eq, PartialEq)]
#[non_exhaustive]
pub enum BackgroundProcessOutcome {
    /// The process exited normally with the given platform exit code.
    Exited(i32),
    /// The process was explicitly stopped; a platform exit code may exist.
    Stopped(Option<i32>),
    /// Completion could not be observed reliably after ownership was retained.
    Dead,
}

impl fmt::Debug for BackgroundProcessOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Exited(_) => "Exited(..)",
            Self::Stopped(_) => "Stopped(..)",
            Self::Dead => "Dead",
        })
    }
}

/// One terminal replacement for an initially published running record.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct BackgroundCompletionRecord {
    updated_at_ms: u64,
    outcome: BackgroundProcessOutcome,
}

impl fmt::Debug for BackgroundCompletionRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackgroundCompletionRecord")
            .field("outcome", &self.outcome)
            .finish_non_exhaustive()
    }
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
/// atomically installs a complete private `running` record without clobbering
/// an existing record. A failure before installation publishes nothing. A
/// failure synchronizing the parent directory after installation may leave
/// exactly one complete valid `running` record at the reserved identity; it
/// must never expose partial record bytes. The caller drops the prepared
/// process and releases the lease, after which the next successful bounded
/// startup reconciliation replaces that unlocked record with `stale`.
/// Returning success is the commit barrier before process release.
/// Constructing either returned future is inert.
pub trait BackgroundRecordLease: Send + Sync + 'static {
    /// Returns the durably reserved nonzero numeric ID.
    fn id(&self) -> u64;

    /// Publishes the complete initial running record atomically.
    ///
    /// # Errors
    ///
    /// A failure before atomic installation must publish nothing. A failure
    /// synchronizing directories after installation may leave the one complete
    /// valid record described by the trait contract.
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
/// that may open the barrier. It observes the operation cancellation token
/// until that irreversible point. On a release error the implementation
/// retains the same no-command-executed guarantee and completes cleanup before
/// returning.
/// `abort` is the fallible proof used after an initial record might have been
/// published: only its success permits that record to become `stopped`.
pub trait PreparedBackgroundProcess: Send + 'static {
    /// Returns a display-only process ID, when the platform exposes one.
    fn pid(&self) -> Option<NonZeroU32>;

    /// Opens the execution barrier and transfers the owned process handle.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled` only when cancellation won before the barrier opened
    /// and complete prepared-resource cleanup was proven. Returns `Process`
    /// when release or cleanup failed or their result is ambiguous.
    fn release(
        self: Box<Self>,
        cancellation: &CancellationToken,
    ) -> Result<Box<dyn OwnedBackgroundProcess>, BackgroundStartError>;

    /// Closes the execution barrier, terminates, and reaps the prepared child.
    ///
    /// The default is deliberately conservative for implementations that only
    /// provide infallible `Drop` cleanup: it performs that cleanup but reports
    /// ambiguity, so a possibly published record cannot claim `stopped`
    /// without an explicit success result.
    ///
    /// # Errors
    ///
    /// Returns a fixed process failure whenever complete cleanup cannot be
    /// proven. The implementation must still make its best cleanup attempt
    /// before returning.
    fn abort(self: Box<Self>) -> Result<(), BackgroundStartError> {
        drop(self);
        Err(BackgroundStartError::new(BackgroundStartErrorKind::Process))
    }
}

/// A released native process whose ownership is sufficient to observe its
/// completion without PID-based lookup.
pub trait OwnedBackgroundProcess: Send + 'static {
    /// Returns a display-only process ID, when the platform exposes one.
    fn pid(&self) -> Option<NonZeroU32>;

    /// Consumes ownership and waits for one terminal observation.
    ///
    /// Future construction must be inert. Cancelling `stop` requests owned
    /// process-resource termination and reap and must resolve as `Stopped`
    /// only once the implementation's documented bounded ownership set is
    /// fully discharged. Dropping the wait future must perform the same
    /// cleanup; it must never detach, leak, or transfer a resource in that
    /// ownership set.
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
    /// infallible ownership transfer. Cancellation can win until native release
    /// opens the execution barrier; it then drops and cleans the prepared
    /// process. Once that barrier opens, cancellation cannot revoke ownership
    /// or change the returned result.
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

    let prepared = await_prepared_or_cancel(
        spawner.prepare(&request, cancellation.clone()),
        &cancellation,
    )
    .await?;
    let prepared = ensure_prepared_not_cancelled(prepared, &cancellation)?;

    let pid = prepared.pid();
    let record = BackgroundRunningRecord::new(id, started_at_ms, &request, pid);
    match await_commit_or_cancel(lease.publish_initial(&record), &cancellation).await {
        Ok(()) => {}
        Err(CommitAwaitError::CancelledBeforePoll) => {
            return Err(abort_prepared(
                prepared,
                BackgroundStartError::new(BackgroundStartErrorKind::Cancelled),
            ));
        }
        Err(CommitAwaitError::CancelledAfterPoll) => {
            return cleanup_after_possible_publication(
                prepared,
                lease.as_ref(),
                &record,
                BackgroundStartError::new(BackgroundStartErrorKind::Cancelled),
            )
            .await;
        }
        Err(CommitAwaitError::Operation(error)) => {
            // A persistence failure can mean that atomic installation succeeded
            // but its directory synchronization failed. If cancellation is now
            // observable, conservatively replace that possibly installed record,
            // while preserving the already-observed operation failure.
            if cancellation.is_cancelled() {
                return cleanup_after_possible_publication(
                    prepared,
                    lease.as_ref(),
                    &record,
                    error,
                )
                .await;
            }
            return Err(error);
        }
    }
    if cancellation.is_cancelled() {
        return cleanup_after_possible_publication(
            prepared,
            lease.as_ref(),
            &record,
            BackgroundStartError::new(BackgroundStartErrorKind::Cancelled),
        )
        .await;
    }

    // Opening the execution barrier is the irreversible commit point. Release
    // observes the same operation token while performing bounded pre-open work;
    // cancellation cannot revoke ownership after the barrier opens.
    let process = match prepared.release(&cancellation) {
        Ok(process) => process,
        Err(error) => {
            let outcome = if error.kind() == BackgroundStartErrorKind::Cancelled {
                BackgroundProcessOutcome::Stopped(None)
            } else {
                BackgroundProcessOutcome::Dead
            };
            if let Ok(completion) = record.completion(started_at_ms, outcome) {
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

async fn cleanup_after_possible_publication(
    prepared: Box<dyn PreparedBackgroundProcess>,
    lease: &dyn BackgroundRecordLease,
    record: &BackgroundRunningRecord,
    primary: BackgroundStartError,
) -> Result<BackgroundHandle, BackgroundStartError> {
    let (outcome, result) = match prepared.abort() {
        Ok(()) => (BackgroundProcessOutcome::Stopped(None), primary),
        Err(error) => {
            let result = if primary.kind() == BackgroundStartErrorKind::Persistence {
                primary
            } else {
                error
            };
            (BackgroundProcessOutcome::Dead, result)
        }
    };
    if let Ok(completion) = record.completion(record.started_at_ms(), outcome) {
        let _ = lease.publish_completion(record, &completion).await;
    }
    Err(result)
}

fn abort_prepared(
    prepared: Box<dyn PreparedBackgroundProcess>,
    primary: BackgroundStartError,
) -> BackgroundStartError {
    match prepared.abort() {
        Ok(()) => primary,
        Err(error) => error,
    }
}

fn ensure_prepared_not_cancelled(
    prepared: Box<dyn PreparedBackgroundProcess>,
    cancellation: &CancellationToken,
) -> Result<Box<dyn PreparedBackgroundProcess>, BackgroundStartError> {
    if cancellation.is_cancelled() {
        Err(abort_prepared(
            prepared,
            BackgroundStartError::new(BackgroundStartErrorKind::Cancelled),
        ))
    } else {
        Ok(prepared)
    }
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
    let mut operation_polled = false;
    std::future::poll_fn(|context| {
        // Cancellation wins before the operation's first poll so an already
        // cancelled request cannot exercise authority. Once started, poll the
        // operation first: independent operation and cancellation wakeups may
        // both arrive before the executor repolls this selector, and a ready
        // operation failure must not be hidden by cancellation.
        if !operation_polled {
            if cancellation_wait.as_mut().poll(context).is_ready() {
                return Poll::Ready(Err(BackgroundStartError::new(
                    BackgroundStartErrorKind::Cancelled,
                )));
            }
            operation_polled = true;
        }
        match operation.as_mut().poll(context) {
            // An operation failure is stronger evidence than cancellation
            // observed during that same poll. In particular, preparation must
            // not report clean cancellation when native cleanup itself failed.
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Ready(Ok(value)) if cancellation.is_cancelled() => {
                drop(value);
                Poll::Ready(Err(BackgroundStartError::new(
                    BackgroundStartErrorKind::Cancelled,
                )))
            }
            Poll::Ready(Ok(value)) => Poll::Ready(Ok(value)),
            Poll::Pending => {
                if cancellation_wait.as_mut().poll(context).is_ready() {
                    Poll::Ready(Err(BackgroundStartError::new(
                        BackgroundStartErrorKind::Cancelled,
                    )))
                } else {
                    Poll::Pending
                }
            }
        }
    })
    .await
}

async fn await_prepared_or_cancel(
    mut operation: BoxFuture<'_, Result<Box<dyn PreparedBackgroundProcess>, BackgroundStartError>>,
    cancellation: &CancellationToken,
) -> Result<Box<dyn PreparedBackgroundProcess>, BackgroundStartError> {
    let mut cancellation_wait = Box::pin(cancellation.cancelled());
    let mut operation_polled = false;
    std::future::poll_fn(|context| {
        if !operation_polled {
            if cancellation_wait.as_mut().poll(context).is_ready() {
                return Poll::Ready(Err(BackgroundStartError::new(
                    BackgroundStartErrorKind::Cancelled,
                )));
            }
            operation_polled = true;
        }
        match operation.as_mut().poll(context) {
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Ready(Ok(prepared)) if cancellation.is_cancelled() => {
                Poll::Ready(Err(abort_prepared(
                    prepared,
                    BackgroundStartError::new(BackgroundStartErrorKind::Cancelled),
                )))
            }
            Poll::Ready(Ok(prepared)) => Poll::Ready(Ok(prepared)),
            Poll::Pending => {
                if cancellation_wait.as_mut().poll(context).is_ready() {
                    Poll::Ready(Err(BackgroundStartError::new(
                        BackgroundStartErrorKind::Cancelled,
                    )))
                } else {
                    Poll::Pending
                }
            }
        }
    })
    .await
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommitAwaitError {
    CancelledBeforePoll,
    CancelledAfterPoll,
    Operation(BackgroundStartError),
}

async fn await_commit_or_cancel<T>(
    mut operation: BoxFuture<'_, Result<T, BackgroundStartError>>,
    cancellation: &CancellationToken,
) -> Result<T, CommitAwaitError> {
    let mut cancellation_wait = Box::pin(cancellation.cancelled());
    let mut operation_polled = false;
    std::future::poll_fn(|context| {
        if !operation_polled {
            if cancellation_wait.as_mut().poll(context).is_ready() {
                return Poll::Ready(Err(CommitAwaitError::CancelledBeforePoll));
            }
            operation_polled = true;
        }
        match operation.as_mut().poll(context) {
            // Once publication has been polled, its failure is primary even if
            // cancellation becomes ready in the same scheduling window.
            Poll::Ready(Err(error)) => Poll::Ready(Err(CommitAwaitError::Operation(error))),
            Poll::Ready(Ok(value)) if cancellation.is_cancelled() => {
                drop(value);
                Poll::Ready(Err(CommitAwaitError::CancelledAfterPoll))
            }
            Poll::Ready(Ok(value)) => Poll::Ready(Ok(value)),
            Poll::Pending => {
                if cancellation_wait.as_mut().poll(context).is_ready() {
                    Poll::Ready(Err(CommitAwaitError::CancelledAfterPoll))
                } else {
                    Poll::Pending
                }
            }
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
        CommitAwaitError, MAX_BACKGROUND_COMMAND_BYTES, MAX_BACKGROUND_CWD_BYTES,
        OwnedBackgroundProcess, PreparedBackgroundProcess, await_commit_or_cancel, await_or_cancel,
        await_prepared_or_cancel,
    };
    use crate::{BoxFuture, CancellationToken};
    use core::future::Future;
    use core::num::NonZeroU32;
    use core::pin::Pin;
    use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use core::task::{Context, Poll};
    use futures_executor::block_on;
    use futures_util::task::noop_waker_ref;
    use std::sync::{Arc, Mutex};
    use std::task::{Wake, Waker};

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
        abort: bool,
    }

    #[derive(Debug, Default)]
    struct Observations {
        events: Mutex<Vec<&'static str>>,
        aborted: AtomicUsize,
        executed: AtomicUsize,
        retained: AtomicUsize,
        completion_publications: AtomicUsize,
        dead_completion_publications: AtomicUsize,
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
                if completion.outcome() == BackgroundProcessOutcome::Dead {
                    self.observations
                        .dead_completion_publications
                        .fetch_add(1, Ordering::Relaxed);
                }
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

    #[derive(Clone, Copy, Debug)]
    enum ScriptedPublishResult {
        Pending,
        Persistence,
        Process,
    }

    #[derive(Debug)]
    struct ScriptedPublishStore {
        observations: Arc<Observations>,
        cancellation: CancellationToken,
        result: ScriptedPublishResult,
    }

    impl BackgroundStore for ScriptedPublishStore {
        fn reserve(
            &self,
        ) -> BoxFuture<'_, Result<Box<dyn BackgroundRecordLease>, BackgroundStartError>> {
            Box::pin(async move {
                self.observations.event("reserve");
                Ok(Box::new(ScriptedPublishLease {
                    observations: Arc::clone(&self.observations),
                    cancellation: self.cancellation.clone(),
                    result: self.result,
                }) as Box<dyn BackgroundRecordLease>)
            })
        }
    }

    #[derive(Debug)]
    struct ScriptedPublishLease {
        observations: Arc<Observations>,
        cancellation: CancellationToken,
        result: ScriptedPublishResult,
    }

    impl BackgroundRecordLease for ScriptedPublishLease {
        fn id(&self) -> u64 {
            7
        }

        fn publish_initial<'a>(
            &'a self,
            record: &'a BackgroundRunningRecord,
        ) -> BoxFuture<'a, Result<(), BackgroundStartError>> {
            Box::pin(ScriptedPublishFuture {
                lease: self,
                record,
                first_poll: true,
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
                if completion.outcome() == BackgroundProcessOutcome::Dead {
                    self.observations
                        .dead_completion_publications
                        .fetch_add(1, Ordering::Relaxed);
                }
                assert_eq!(initial.id(), 7);
                assert_eq!(completion.updated_at_ms(), 1_234);
                assert!(matches!(
                    completion.outcome(),
                    BackgroundProcessOutcome::Dead | BackgroundProcessOutcome::Stopped(None)
                ));
                Ok(())
            })
        }
    }

    #[derive(Debug)]
    struct ScriptedPublishFuture<'a> {
        lease: &'a ScriptedPublishLease,
        record: &'a BackgroundRunningRecord,
        first_poll: bool,
    }

    impl Future for ScriptedPublishFuture<'_> {
        type Output = Result<(), BackgroundStartError>;

        fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
            assert!(self.first_poll, "scripted publication must not be repolled");
            self.first_poll = false;
            self.lease.observations.event("publish");
            assert_eq!(self.record.id(), 7);
            assert_eq!(self.record.started_at_ms(), 1_234);
            assert_eq!(self.record.command(), "echo ready");
            assert_eq!(self.record.cwd(), "/workspace");

            // Model atomic installation before the native adapter either
            // yields or reports the permitted directory-sync ambiguity.
            self.lease.cancellation.cancel();
            match self.lease.result {
                ScriptedPublishResult::Pending => Poll::Pending,
                ScriptedPublishResult::Persistence => {
                    Poll::Ready(Err(error(BackgroundStartErrorKind::Persistence)))
                }
                ScriptedPublishResult::Process => {
                    Poll::Ready(Err(error(BackgroundStartErrorKind::Process)))
                }
            }
        }
    }

    #[derive(Debug)]
    #[allow(
        clippy::struct_excessive_bools,
        reason = "test double independently injects each process lifecycle boundary"
    )]
    struct FakeSpawner {
        observations: Arc<Observations>,
        prepare_fail: bool,
        release_fail: bool,
        abort_fail: bool,
        cancel_during_prepare: Option<CancellationToken>,
        cancel_during_pid: Option<CancellationToken>,
        cancel_during_release: Option<CancellationToken>,
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
                if let Some(cancellation) = &self.cancel_during_prepare {
                    cancellation.cancel();
                }
                if self.prepare_fail {
                    return Err(error(BackgroundStartErrorKind::Process));
                }
                let prepared: Box<dyn PreparedBackgroundProcess> = Box::new(FakePrepared {
                    observations: Arc::clone(&self.observations),
                    released: false,
                    release_fail: self.release_fail,
                    abort_fail: self.abort_fail,
                    cancel_during_pid: self.cancel_during_pid.clone(),
                    cancel_during_release: self.cancel_during_release.clone(),
                });
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

    #[derive(Debug, Default)]
    struct PollGate {
        ready: AtomicBool,
        polls: AtomicUsize,
        waker: Mutex<Option<Waker>>,
    }

    impl PollGate {
        fn open(&self) {
            self.ready.store(true, Ordering::Release);
            let waker = self
                .waker
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }

    #[derive(Debug)]
    struct GatedFuture<T> {
        output: Option<T>,
        gate: Arc<PollGate>,
    }

    impl<T> GatedFuture<T> {
        fn new(output: T, gate: Arc<PollGate>) -> Self {
            Self {
                output: Some(output),
                gate,
            }
        }
    }

    impl<T: Unpin> Future for GatedFuture<T> {
        type Output = T;

        fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
            self.gate.polls.fetch_add(1, Ordering::Relaxed);
            if self.gate.ready.load(Ordering::Acquire) {
                return Poll::Ready(self.output.take().expect("gated future resolves once"));
            }

            let superseded = self
                .gate
                .waker
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .replace(context.waker().clone());
            drop(superseded);

            if self.gate.ready.load(Ordering::Acquire) {
                Poll::Ready(self.output.take().expect("gated future resolves once"))
            } else {
                Poll::Pending
            }
        }
    }

    #[derive(Debug, Default)]
    struct WakeCounter(AtomicUsize);

    impl Wake for WakeCounter {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[derive(Debug)]
    struct FakePrepared {
        observations: Arc<Observations>,
        released: bool,
        release_fail: bool,
        abort_fail: bool,
        cancel_during_pid: Option<CancellationToken>,
        cancel_during_release: Option<CancellationToken>,
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
            if let Some(cancellation) = &self.cancel_during_pid {
                cancellation.cancel();
            }
            NonZeroU32::new(42)
        }

        fn release(
            mut self: Box<Self>,
            _cancellation: &CancellationToken,
        ) -> Result<Box<dyn OwnedBackgroundProcess>, BackgroundStartError> {
            self.observations.event("release");
            if let Some(cancellation) = &self.cancel_during_release {
                cancellation.cancel();
                self.released = true;
                return Err(error(BackgroundStartErrorKind::Cancelled));
            }
            if self.release_fail {
                self.released = true;
                return Err(error(BackgroundStartErrorKind::Process));
            }
            self.observations.executed.fetch_add(1, Ordering::Relaxed);
            self.released = true;
            Ok(Box::new(FakeOwned))
        }

        fn abort(mut self: Box<Self>) -> Result<(), BackgroundStartError> {
            self.observations.aborted.fetch_add(1, Ordering::Relaxed);
            self.observations.event("abort");
            self.released = true;
            if self.abort_fail {
                Err(error(BackgroundStartErrorKind::Process))
            } else {
                Ok(())
            }
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
                abort_fail: failures.abort,
                cancel_during_prepare: (cancel_stage == Some("prepare"))
                    .then_some(cancellation.clone())
                    .flatten(),
                cancel_during_pid: (cancel_stage == Some("pid"))
                    .then_some(cancellation.clone())
                    .flatten(),
                cancel_during_release: (cancel_stage == Some("release"))
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

    fn scripted_publication_fixture(
        result: ScriptedPublishResult,
        cancellation: CancellationToken,
    ) -> (BackgroundSupervisor, Arc<Observations>) {
        scripted_publication_fixture_with_abort(result, cancellation, false)
    }

    fn scripted_publication_fixture_with_abort(
        result: ScriptedPublishResult,
        cancellation: CancellationToken,
        abort_fail: bool,
    ) -> (BackgroundSupervisor, Arc<Observations>) {
        let observations = Arc::new(Observations::default());
        let supervisor = BackgroundSupervisor::new(
            Arc::new(FakeClock {
                observations: Arc::clone(&observations),
                fail: false,
            }),
            Arc::new(ScriptedPublishStore {
                observations: Arc::clone(&observations),
                cancellation,
                result,
            }),
            Arc::new(FakeSpawner {
                observations: Arc::clone(&observations),
                prepare_fail: false,
                release_fail: false,
                abort_fail,
                cancel_during_prepare: None,
                cancel_during_pid: None,
                cancel_during_release: None,
                stay_pending: false,
                pending_dropped: Arc::new(AtomicUsize::new(0)),
            }),
            Arc::new(FakeRetainer {
                observations: Arc::clone(&observations),
                capacity_fail: false,
                cancel_during_retain: None,
            }),
        );
        (supervisor, observations)
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
    fn process_outcomes_and_completion_do_not_debug_payloads() {
        const EXIT_SENTINEL: i32 = -847_362_901;
        const TIMESTAMP_SENTINEL: u64 = 1_735_689_421_987;

        assert_eq!(
            format!("{:?}", BackgroundProcessOutcome::Exited(EXIT_SENTINEL)),
            "Exited(..)"
        );
        assert_eq!(
            format!(
                "{:?}",
                BackgroundProcessOutcome::Stopped(Some(EXIT_SENTINEL))
            ),
            "Stopped(..)"
        );
        assert_eq!(
            format!("{:?}", BackgroundProcessOutcome::Stopped(None)),
            "Stopped(..)"
        );
        assert_eq!(format!("{:?}", BackgroundProcessOutcome::Dead), "Dead");

        let completion = BackgroundCompletionRecord {
            updated_at_ms: TIMESTAMP_SENTINEL,
            outcome: BackgroundProcessOutcome::Exited(EXIT_SENTINEL),
        };
        let debug = format!("{completion:?}");
        assert_eq!(
            debug,
            "BackgroundCompletionRecord { outcome: Exited(..), .. }"
        );
        assert!(!debug.contains(&EXIT_SENTINEL.to_string()));
        assert!(!debug.contains(&TIMESTAMP_SENTINEL.to_string()));
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
    fn selector_cancellation_before_first_poll_does_not_poll_the_operation() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let gate = Arc::new(PollGate::default());
        let operation = Box::pin(GatedFuture::new(
            Ok::<(), BackgroundStartError>(()),
            Arc::clone(&gate),
        ));
        let mut selector = Box::pin(await_or_cancel(operation, &cancellation));
        let mut context = Context::from_waker(noop_waker_ref());

        let result = selector.as_mut().poll(&mut context);

        assert!(matches!(
            result,
            Poll::Ready(Err(error)) if error.kind() == BackgroundStartErrorKind::Cancelled
        ));
        assert_eq!(gate.polls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn started_operation_failure_precedes_cancellation_when_both_wake_before_repoll() {
        let cancellation = CancellationToken::new();
        let gate = Arc::new(PollGate::default());
        let operation = Box::pin(GatedFuture::new(
            Err::<(), BackgroundStartError>(error(BackgroundStartErrorKind::Persistence)),
            Arc::clone(&gate),
        ));
        let mut selector = Box::pin(await_or_cancel(operation, &cancellation));
        let wake_counter = Arc::new(WakeCounter::default());
        let waker = Waker::from(Arc::clone(&wake_counter));
        let mut context = Context::from_waker(&waker);

        assert!(matches!(
            selector.as_mut().poll(&mut context),
            Poll::Pending
        ));
        gate.open();
        cancellation.cancel();
        assert_eq!(wake_counter.0.load(Ordering::Relaxed), 2);

        assert!(matches!(
            selector.as_mut().poll(&mut context),
            Poll::Ready(Err(error)) if error.kind() == BackgroundStartErrorKind::Persistence
        ));
        assert_eq!(gate.polls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn ready_prepared_abort_failure_precedes_cancellation_after_two_wakeups() {
        let cancellation = CancellationToken::new();
        let observations = Arc::new(Observations::default());
        let gate = Arc::new(PollGate::default());
        let prepared: Box<dyn PreparedBackgroundProcess> = Box::new(FakePrepared {
            observations: Arc::clone(&observations),
            released: false,
            release_fail: false,
            abort_fail: true,
            cancel_during_pid: None,
            cancel_during_release: None,
        });
        let operation = Box::pin(GatedFuture::new(
            Ok::<Box<dyn PreparedBackgroundProcess>, BackgroundStartError>(prepared),
            Arc::clone(&gate),
        ));
        let mut selector = Box::pin(await_prepared_or_cancel(operation, &cancellation));
        let wake_counter = Arc::new(WakeCounter::default());
        let waker = Waker::from(Arc::clone(&wake_counter));
        let mut context = Context::from_waker(&waker);

        assert!(matches!(
            selector.as_mut().poll(&mut context),
            Poll::Pending
        ));
        gate.open();
        cancellation.cancel();
        assert_eq!(wake_counter.0.load(Ordering::Relaxed), 2);

        assert!(matches!(
            selector.as_mut().poll(&mut context),
            Poll::Ready(Err(error)) if error.kind() == BackgroundStartErrorKind::Process
        ));
        assert_eq!(gate.polls.load(Ordering::Relaxed), 2);
        assert_eq!(observations.events(), ["abort"]);
        assert_eq!(observations.aborted.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn started_preparation_failure_precedes_cancellation_after_two_wakeups() {
        let cancellation = CancellationToken::new();
        let gate = Arc::new(PollGate::default());
        let operation = Box::pin(GatedFuture::new(
            Err::<Box<dyn PreparedBackgroundProcess>, BackgroundStartError>(error(
                BackgroundStartErrorKind::Process,
            )),
            Arc::clone(&gate),
        ));
        let mut selector = Box::pin(await_prepared_or_cancel(operation, &cancellation));
        let wake_counter = Arc::new(WakeCounter::default());
        let waker = Waker::from(Arc::clone(&wake_counter));
        let mut context = Context::from_waker(&waker);

        assert!(matches!(
            selector.as_mut().poll(&mut context),
            Poll::Pending
        ));
        gate.open();
        cancellation.cancel();
        assert_eq!(wake_counter.0.load(Ordering::Relaxed), 2);

        assert!(matches!(
            selector.as_mut().poll(&mut context),
            Poll::Ready(Err(error)) if error.kind() == BackgroundStartErrorKind::Process
        ));
        assert_eq!(gate.polls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn started_commit_persistence_failure_precedes_cancellation_after_two_wakeups() {
        let cancellation = CancellationToken::new();
        let gate = Arc::new(PollGate::default());
        let operation = Box::pin(GatedFuture::new(
            Err::<(), BackgroundStartError>(error(BackgroundStartErrorKind::Persistence)),
            Arc::clone(&gate),
        ));
        let mut selector = Box::pin(await_commit_or_cancel(operation, &cancellation));
        let wake_counter = Arc::new(WakeCounter::default());
        let waker = Waker::from(Arc::clone(&wake_counter));
        let mut context = Context::from_waker(&waker);

        assert!(matches!(
            selector.as_mut().poll(&mut context),
            Poll::Pending
        ));
        gate.open();
        cancellation.cancel();
        assert_eq!(wake_counter.0.load(Ordering::Relaxed), 2);

        assert!(matches!(
            selector.as_mut().poll(&mut context),
            Poll::Ready(Err(CommitAwaitError::Operation(error)))
                if error.kind() == BackgroundStartErrorKind::Persistence
        ));
        assert_eq!(gate.polls.load(Ordering::Relaxed), 2);
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
    fn same_poll_prepare_cancellation_propagates_abort_failure() {
        let cancellation = CancellationToken::new();
        let failures = Failures {
            abort: true,
            ..Failures::default()
        };
        let (supervisor, observations, _) =
            fixture(failures, Some(cancellation.clone()), Some("prepare"), false);

        let result = block_on(supervisor.start(request(), cancellation));

        assert_eq!(
            result.unwrap_err().kind(),
            BackgroundStartErrorKind::Process
        );
        assert_eq!(
            observations.events(),
            ["admit", "reserve", "clock", "prepare", "abort"]
        );
        assert_eq!(observations.aborted.load(Ordering::Relaxed), 1);
        assert_eq!(observations.executed.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn cancellation_before_publication_poll_propagates_abort_failure() {
        let cancellation = CancellationToken::new();
        let failures = Failures {
            abort: true,
            ..Failures::default()
        };
        let (supervisor, observations, _) =
            fixture(failures, Some(cancellation.clone()), Some("pid"), false);

        let result = block_on(supervisor.start(request(), cancellation));

        assert_eq!(
            result.unwrap_err().kind(),
            BackgroundStartErrorKind::Process
        );
        assert_eq!(
            observations.events(),
            ["admit", "reserve", "clock", "prepare", "abort"]
        );
        assert_eq!(observations.aborted.load(Ordering::Relaxed), 1);
        assert_eq!(
            observations.completion_publications.load(Ordering::Relaxed),
            0
        );
        assert_eq!(observations.executed.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn same_poll_prepare_process_failure_takes_precedence_over_cancellation() {
        let cancellation = CancellationToken::new();
        let failures = Failures {
            prepare: true,
            ..Failures::default()
        };
        let (supervisor, observations, _) =
            fixture(failures, Some(cancellation.clone()), Some("prepare"), false);

        let result = block_on(supervisor.start(request(), cancellation.clone()));

        assert_eq!(
            result.unwrap_err().kind(),
            BackgroundStartErrorKind::Process
        );
        assert!(cancellation.is_cancelled());
        assert_eq!(
            observations.events(),
            ["admit", "reserve", "clock", "prepare"]
        );
        assert_eq!(observations.aborted.load(Ordering::Relaxed), 0);
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
    fn cancellation_after_install_then_pending_replaces_with_stopped() {
        let cancellation = CancellationToken::new();
        let (supervisor, observations) =
            scripted_publication_fixture(ScriptedPublishResult::Pending, cancellation.clone());

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
        assert_eq!(observations.aborted.load(Ordering::Relaxed), 1);
        assert_eq!(
            observations.completion_publications.load(Ordering::Relaxed),
            1
        );
        assert_eq!(observations.executed.load(Ordering::Relaxed), 0);
        assert_eq!(observations.retained.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn same_poll_cancellation_preserves_ambiguous_persistence_and_replaces_with_stopped() {
        let cancellation = CancellationToken::new();
        let (supervisor, observations) =
            scripted_publication_fixture(ScriptedPublishResult::Persistence, cancellation.clone());

        let result = block_on(supervisor.start(request(), cancellation.clone()));

        assert_eq!(
            result.unwrap_err().kind(),
            BackgroundStartErrorKind::Persistence
        );
        assert!(cancellation.is_cancelled());
        assert_eq!(
            observations.events(),
            [
                "admit", "reserve", "clock", "prepare", "publish", "abort", "complete"
            ]
        );
        assert_eq!(observations.aborted.load(Ordering::Relaxed), 1);
        assert_eq!(
            observations.completion_publications.load(Ordering::Relaxed),
            1
        );
        assert_eq!(observations.executed.load(Ordering::Relaxed), 0);
        assert_eq!(observations.retained.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn same_poll_persistence_remains_primary_when_abort_requires_dead_replacement() {
        let cancellation = CancellationToken::new();
        let (supervisor, observations) = scripted_publication_fixture_with_abort(
            ScriptedPublishResult::Persistence,
            cancellation.clone(),
            true,
        );

        let result = block_on(supervisor.start(request(), cancellation.clone()));

        assert_eq!(
            result.unwrap_err().kind(),
            BackgroundStartErrorKind::Persistence
        );
        assert!(cancellation.is_cancelled());
        assert_eq!(
            observations.events(),
            [
                "admit", "reserve", "clock", "prepare", "publish", "abort", "complete"
            ]
        );
        assert_eq!(observations.aborted.load(Ordering::Relaxed), 1);
        assert_eq!(
            observations
                .dead_completion_publications
                .load(Ordering::Relaxed),
            1
        );
        assert_eq!(observations.executed.load(Ordering::Relaxed), 0);
        assert_eq!(observations.retained.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn cancellation_does_not_mask_a_stronger_publication_error() {
        let cancellation = CancellationToken::new();
        let (supervisor, observations) =
            scripted_publication_fixture(ScriptedPublishResult::Process, cancellation.clone());

        let result = block_on(supervisor.start(request(), cancellation));

        assert_eq!(
            result.unwrap_err().kind(),
            BackgroundStartErrorKind::Process
        );
        assert_eq!(
            observations.events(),
            [
                "admit", "reserve", "clock", "prepare", "publish", "abort", "complete"
            ]
        );
        assert_eq!(observations.aborted.load(Ordering::Relaxed), 1);
        assert_eq!(
            observations.completion_publications.load(Ordering::Relaxed),
            1
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
    fn cancellation_records_dead_and_propagates_prepared_abort_failure() {
        let cancellation = CancellationToken::new();
        let failures = Failures {
            abort: true,
            ..Failures::default()
        };
        let (supervisor, observations, _) =
            fixture(failures, Some(cancellation.clone()), Some("publish"), false);

        let result = block_on(supervisor.start(request(), cancellation));

        assert_eq!(
            result.unwrap_err().kind(),
            BackgroundStartErrorKind::Process
        );
        assert_eq!(observations.aborted.load(Ordering::Relaxed), 1);
        assert_eq!(
            observations
                .dead_completion_publications
                .load(Ordering::Relaxed),
            1
        );
        assert_eq!(observations.executed.load(Ordering::Relaxed), 0);
        assert_eq!(observations.retained.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn proven_release_cancellation_records_stopped_and_returns_cancelled() {
        let cancellation = CancellationToken::new();
        let (supervisor, observations, _) = fixture(
            Failures::default(),
            Some(cancellation.clone()),
            Some("release"),
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
                "admit", "reserve", "clock", "prepare", "publish", "release", "complete"
            ]
        );
        assert_eq!(
            observations.completion_publications.load(Ordering::Relaxed),
            1
        );
        assert_eq!(
            observations
                .dead_completion_publications
                .load(Ordering::Relaxed),
            0
        );
        assert_eq!(observations.executed.load(Ordering::Relaxed), 0);
        assert_eq!(observations.retained.load(Ordering::Relaxed), 0);
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
