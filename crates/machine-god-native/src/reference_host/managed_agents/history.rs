//! One owned canonical transcript read, separate from truncated journal summaries.
use super::NativeManagedAgents;
use crate::{NativeObservedManagedAgent, managed::store::JournalSnapshot};
use machine_god_core::{BoxFuture, CancellationToken, SessionRecord};
use std::{
    fmt,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Waker},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeManagedHistoryError {
    Busy,
    Closed,
    Stale,
    Cancelled,
    Unavailable,
}
impl fmt::Display for NativeManagedHistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Busy => "managed history capacity occupied",
            Self::Closed => "managed history reader closed",
            Self::Stale => "managed history observation changed",
            Self::Cancelled => "managed history read cancelled",
            Self::Unavailable => "managed history unavailable",
        })
    }
}
impl std::error::Error for NativeManagedHistoryError {}
type Error = NativeManagedHistoryError;

struct Identity;
#[derive(Clone)]
pub struct NativeManagedHistoryRequest {
    cancellation: CancellationToken,
    identity: Weak<Identity>,
    sequence: u64,
}
impl PartialEq for NativeManagedHistoryRequest {
    fn eq(&self, other: &Self) -> bool {
        self.identity.ptr_eq(&other.identity) && self.sequence == other.sequence
    }
}
impl Eq for NativeManagedHistoryRequest {}
impl NativeManagedHistoryRequest {
    pub(crate) fn cancel(&self) {
        self.cancellation.cancel();
    }
}

/// One immutable canonical record, not a runtime, turn or permission capability.
/// Retaining it occupies this manager's history slot, but does not hold cleanup.
pub struct NativeManagedHistorySnapshot {
    record: Arc<SessionRecord>,
    observation: NativeObservedManagedAgent,
    _reservation: Arc<Reservation>,
}
impl NativeManagedHistorySnapshot {
    #[must_use]
    pub fn record(&self) -> &SessionRecord {
        &self.record
    }
    #[must_use]
    pub const fn observation(&self) -> &NativeObservedManagedAgent {
        &self.observation
    }
}
pub struct NativeManagedHistoryOutcome {
    pub request: NativeManagedHistoryRequest,
    pub result: Result<NativeManagedHistorySnapshot, Error>,
}
macro_rules! redacted {
    ($($ty:ty),+ $(,)?) => {$(impl fmt::Debug for $ty {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct(stringify!($ty)).finish_non_exhaustive()
        }
    })+};
}
redacted!(
    NativeManagedHistorySnapshot,
    NativeManagedHistoryRequest,
    NativeManagedHistoryOutcome
);

pub(in crate::reference_host) struct Reservation(Arc<AtomicBool>);
impl Drop for Reservation {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
struct Pending {
    request: NativeManagedHistoryRequest,
    cancellation: CancellationToken,
    future: BoxFuture<'static, Result<NativeManagedHistorySnapshot, Error>>,
}
pub(super) struct Reader {
    identity: Arc<Identity>,
    occupied: Arc<AtomicBool>,
    sequence: u64,
    pending: Option<Pending>,
    outcome: Option<NativeManagedHistoryOutcome>,
    closed: bool,
    wake: Option<Waker>,
}
impl Default for Reader {
    fn default() -> Self {
        Self {
            identity: Arc::new(Identity),
            occupied: Arc::new(AtomicBool::new(false)),
            sequence: 0,
            pending: None,
            outcome: None,
            closed: false,
            wake: None,
        }
    }
}
impl Reader {
    pub(super) fn poll(&mut self, cx: &mut Context<'_>) {
        self.wake = Some(cx.waker().clone());
        let Some(pending) = &mut self.pending else {
            return;
        };
        if let Poll::Ready(result) = pending.future.as_mut().poll(cx) {
            let pending = self.pending.take().expect("retained history request");
            self.outcome = Some(NativeManagedHistoryOutcome {
                request: pending.request,
                result: if self.closed {
                    Err(Error::Closed)
                } else if pending.cancellation.is_cancelled() {
                    Err(Error::Cancelled)
                } else {
                    result
                },
            });
            cx.waker().wake_by_ref();
        }
    }
    pub(super) fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        if let Some(pending) = &self.pending {
            pending.cancellation.cancel();
        }
        // A taken snapshot is detached immutable data; no resource custody is
        // transferred to it. Unconsumed data can be released immediately.
        self.outcome = None;
        self.notify();
    }
    pub(super) fn is_closed(&self) -> bool {
        self.closed
    }
    fn notify(&mut self) {
        if let Some(wake) = self.wake.take() {
            wake.wake();
        }
    }
    pub(super) fn settled(&self) -> bool {
        self.pending.is_none()
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.close();
    }
}

impl NativeManagedAgents {
    /// Queues one exact, bounded canonical transcript observation. Archived
    /// children remain archived: this never prepares a runtime or executes work.
    /// Drive this owner until the correlated outcome becomes available.
    /// # Errors
    /// Rejects foreign observations, shutdown, or occupied read/snapshot capacity.
    pub fn request_history(
        &mut self,
        observed: NativeObservedManagedAgent,
    ) -> Result<NativeManagedHistoryRequest, Error> {
        if self.history.closed || self.manager.is_closing() {
            return Err(Error::Closed);
        }
        if !self.manager.owns_observation(&observed) {
            return Err(Error::Stale);
        }
        if self.history.pending.is_some() || self.history.outcome.is_some() {
            return Err(Error::Busy);
        }
        let sequence = self
            .history
            .sequence
            .checked_add(1)
            .ok_or(Error::Unavailable)?;
        self.history
            .occupied
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::Busy)?;
        let reservation = Arc::new(Reservation(self.history.occupied.clone()));
        let resident = self
            .observed_runtime(&observed)
            .map(|runtime| runtime.record_snapshot());
        let cancellation = CancellationToken::new();
        let future = self.factory.read_history(
            self.journal.clone(),
            observed,
            resident,
            reservation,
            cancellation.clone(),
        );
        let request = NativeManagedHistoryRequest {
            cancellation: cancellation.clone(),
            identity: Arc::downgrade(&self.history.identity),
            sequence,
        };
        self.history.sequence = sequence;
        self.history.pending = Some(Pending {
            request: request.clone(),
            cancellation,
            future,
        });
        self.history.notify();
        Ok(request)
    }

    /// Cancels only the original pending read, retaining worker settlement.
    pub fn cancel_history(&mut self, request: &NativeManagedHistoryRequest) -> bool {
        if let Some(pending) = &self.history.pending
            && &pending.request == request
        {
            pending.cancellation.cancel();
            self.history.notify();
            true
        } else {
            false
        }
    }

    #[must_use]
    pub fn take_history_outcome(&mut self) -> Option<NativeManagedHistoryOutcome> {
        self.history.outcome.take()
    }
}

pub(in crate::reference_host) fn validate(
    observed: &NativeObservedManagedAgent,
    snapshot: &JournalSnapshot,
) -> Result<(), Error> {
    if observed.matches_snapshot(snapshot) {
        Ok(())
    } else {
        Err(Error::Stale)
    }
}
pub(in crate::reference_host) fn snapshot(
    record: Arc<SessionRecord>,
    observation: NativeObservedManagedAgent,
    original: &JournalSnapshot,
    reservation: Arc<Reservation>,
) -> Result<NativeManagedHistorySnapshot, Error> {
    if record.id != original.head.transcript.session_id
        || record.incarnation_id != original.head.transcript.incarnation
    {
        return Err(Error::Stale);
    }
    Ok(NativeManagedHistorySnapshot {
        record,
        observation,
        _reservation: reservation,
    })
}
