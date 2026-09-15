//! Candidate headroom is reserved before runtime effects and retained to handoff.
use super::{Arc, Context, ManagedManager, ManagedRuntimeError, Poll, Weak};
use futures_util::task::AtomicWaker;
use std::sync::atomic::{AtomicU8, Ordering};

const WAITING: u8 = 0;
const GRANTED: u8 = 1;
const CONSUMED: u8 = 2;

pub(super) struct State {
    phase: AtomicU8,
    ready: AtomicWaker,
    manager: Weak<AtomicWaker>,
}

/// Non-cloneable original allocation ticket. It owns no runtime or host service.
pub(crate) struct ManagedForegroundReservation(Arc<State>);
impl Drop for ManagedForegroundReservation {
    fn drop(&mut self) {
        self.0.phase.store(CONSUMED, Ordering::Release);
        if let Some(wake) = self.0.manager.upgrade() {
            wake.wake();
        }
    }
}

impl ManagedManager {
    pub(crate) fn reserve_foreground(
        &mut self,
    ) -> Result<ManagedForegroundReservation, ManagedRuntimeError> {
        if self.closing {
            return Err(ManagedRuntimeError::Unavailable);
        }
        self.foreground_reservations.retain(|state| {
            state
                .upgrade()
                .is_some_and(|state| state.phase.load(Ordering::Acquire) != CONSUMED)
        });
        if self.foreground_reservations.len() >= self.limits.residents {
            return Err(ManagedRuntimeError::Capacity);
        }
        let state = Arc::new(State {
            phase: AtomicU8::new(WAITING),
            ready: AtomicWaker::new(),
            manager: Arc::downgrade(&self.reservation_wake),
        });
        self.foreground_reservations.push(Arc::downgrade(&state));
        self.reservation_wake.wake();
        Ok(ManagedForegroundReservation(state))
    }

    pub(crate) fn poll_foreground_reservation(
        &self,
        reservation: &ManagedForegroundReservation,
        cx: &Context<'_>,
    ) -> Poll<Result<(), ManagedRuntimeError>> {
        if self.closing {
            return Poll::Ready(Err(ManagedRuntimeError::Unavailable));
        }
        if !reservation
            .0
            .manager
            .ptr_eq(&Arc::downgrade(&self.reservation_wake))
        {
            return Poll::Ready(Err(ManagedRuntimeError::Invalid));
        }
        reservation.0.ready.register(cx.waker());
        match reservation.0.phase.load(Ordering::Acquire) {
            WAITING => Poll::Pending,
            GRANTED => Poll::Ready(Ok(())),
            _ => Poll::Ready(Err(ManagedRuntimeError::Invalid)),
        }
    }

    pub(super) fn consume_foreground_reservation(
        &self,
        reservation: &ManagedForegroundReservation,
    ) -> Result<(), ManagedRuntimeError> {
        if !reservation
            .0
            .manager
            .ptr_eq(&Arc::downgrade(&self.reservation_wake))
        {
            return Err(ManagedRuntimeError::Invalid);
        }
        reservation
            .0
            .phase
            .compare_exchange(GRANTED, CONSUMED, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| ManagedRuntimeError::Invalid)
    }

    pub(super) fn has_foreground_reservations(&self) -> bool {
        self.foreground_reservations
            .iter()
            .filter_map(Weak::upgrade)
            .any(|state| state.phase.load(Ordering::Acquire) != CONSUMED)
    }

    pub(super) fn validate_foreground_reservation(
        &self,
        reservation: &ManagedForegroundReservation,
    ) -> Result<(), ManagedRuntimeError> {
        if reservation
            .0
            .manager
            .ptr_eq(&Arc::downgrade(&self.reservation_wake))
            && reservation.0.phase.load(Ordering::Acquire) == GRANTED
        {
            Ok(())
        } else {
            Err(ManagedRuntimeError::Invalid)
        }
    }

    pub(super) fn reserved_foregrounds(&self) -> usize {
        self.foreground_reservations
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|state| state.phase.load(Ordering::Acquire) == GRANTED)
            .count()
    }

    pub(super) fn waiting_foreground(&self) -> bool {
        self.foreground_reservations
            .iter()
            .filter_map(Weak::upgrade)
            .any(|state| state.phase.load(Ordering::Acquire) == WAITING)
    }

    pub(super) fn wake_foreground_reservations(&self) {
        for state in self
            .foreground_reservations
            .iter()
            .filter_map(Weak::upgrade)
        {
            state.ready.wake();
        }
    }

    pub(super) fn poll_foreground_reservations(&mut self, cx: &Context<'_>) -> bool {
        self.reservation_wake.register(cx.waker());
        // Commands may hold an in-flight factory allocation. They settle before
        // a candidate can acquire headroom; no second allocator races their slot.
        if self.closing || self.active.is_some() {
            return false;
        }
        let mut progress = false;
        for state in self
            .foreground_reservations
            .iter()
            .filter_map(Weak::upgrade)
        {
            if state.phase.load(Ordering::Acquire) == WAITING
                && self.has_capacity()
                && state
                    .phase
                    .compare_exchange(WAITING, GRANTED, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                state.ready.wake();
                progress = true;
            }
        }
        progress
    }
}
