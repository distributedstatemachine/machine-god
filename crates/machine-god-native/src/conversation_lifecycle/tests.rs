use super::*;
use futures_executor::block_on;
use std::{
    sync::{
        Weak,
        atomic::{AtomicUsize, Ordering},
    },
    task::Wake,
};

struct ReentrantWake {
    gate: Weak<LifecycleGate>,
    wakes: Arc<AtomicUsize>,
}
impl Wake for ReentrantWake {
    fn wake(self: Arc<Self>) {
        if let Some(gate) = self.gate.upgrade() {
            let _ = gate.phase();
        }
        self.wakes.fetch_add(1, Ordering::SeqCst);
    }
}
impl Drop for ReentrantWake {
    fn drop(&mut self) {
        if let Some(gate) = self.gate.upgrade() {
            let _ = gate.phase();
        }
    }
}

#[test]
fn idle_read_witness_requires_exact_live_generation_and_no_permits() {
    let gate = LifecycleGate::new();
    let other = LifecycleGate::new();
    let permit = gate.acquire().unwrap();
    let guard = gate.begin_quiescence().unwrap();
    assert!(guard.belongs_to(&gate));
    assert!(!guard.belongs_to(&other));
    assert_eq!(guard.check_idle(), Err(LifecycleError::Busy));
    drop(permit);
    assert_eq!(guard.check_idle(), Ok(()));
    gate.state.lock().unwrap().generation += 1;
    assert_eq!(guard.check_idle(), Err(LifecycleError::Stale));
    gate.state.lock().unwrap().phase = LifecyclePhase::Retired;
    assert_eq!(guard.check_idle(), Err(LifecycleError::Stale));
    drop(guard);
    assert_eq!(gate.phase(), LifecyclePhase::Retired);
}

#[test]
fn exact_permits_bound_and_retirement_is_irreversible() {
    let gate = LifecycleGate::new();
    let other = LifecycleGate::new();
    let permits = (0..MAX_PERMITS)
        .map(|_| gate.acquire().unwrap())
        .collect::<Vec<_>>();
    assert!(permits[0].belongs_to(&gate));
    assert!(!permits[0].belongs_to(&other));
    assert!(matches!(gate.acquire(), Err(LifecycleError::Busy)));
    let mut guard = gate.begin_quiescence().unwrap();
    assert!(matches!(gate.acquire(), Err(LifecycleError::Quiescing)));
    drop(permits);
    block_on(guard.wait_idle()).unwrap();
    guard.retire().unwrap();
    assert_eq!(gate.phase(), LifecyclePhase::Retired);
    assert!(matches!(gate.acquire(), Err(LifecycleError::Retired)));
    assert!(matches!(
        gate.begin_quiescence(),
        Err(LifecycleError::Retired)
    ));
}

#[test]
fn dropped_fence_and_failed_busy_retirement_reopen_without_stale_rollback() {
    let gate = LifecycleGate::new();
    let permit = gate.acquire().unwrap();
    let guard = gate.begin_quiescence().unwrap();
    assert!(matches!(
        gate.begin_quiescence(),
        Err(LifecycleError::Quiescing)
    ));
    assert_eq!(guard.retire(), Err(LifecycleError::Busy));
    assert_eq!(gate.phase(), LifecyclePhase::Open);
    let old = gate.begin_quiescence().unwrap();
    // Simulate a stale ticket: only the matching owner may reopen a phase.
    gate.state.lock().unwrap().generation += 1;
    drop(old);
    assert_eq!(gate.phase(), LifecyclePhase::Quiescing);
    drop(permit);
}

#[test]
fn waiter_is_inert_bounded_dropped_and_reentrant_woken_outside_lock() {
    let gate = LifecycleGate::new();
    let permit = gate.acquire().unwrap();
    let mut guard = gate.begin_quiescence().unwrap();
    drop(guard.wait_idle());
    assert!(gate.state.lock().unwrap().waiter.is_none());
    let count = Arc::new(AtomicUsize::new(0));
    let waker = Waker::from(Arc::new(ReentrantWake {
        gate: Arc::downgrade(&gate),
        wakes: count.clone(),
    }));
    let mut cx = Context::from_waker(&waker);
    let mut wait = guard.wait_idle();
    assert!(wait.as_mut().poll(&mut cx).is_pending());
    assert!(wait.as_mut().poll(&mut cx).is_pending());
    drop(wait);
    assert!(gate.state.lock().unwrap().waiter.is_none());
    assert_eq!(gate.phase(), LifecyclePhase::Quiescing);
    let mut wait = guard.wait_idle();
    assert!(wait.as_mut().poll(&mut cx).is_pending());
    drop(permit);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(wait.as_mut().poll(&mut cx), Poll::Ready(Ok(())));
    drop(wait);
    drop(guard);
    drop(waker);
    assert_eq!(gate.phase(), LifecyclePhase::Open);
}

#[test]
fn identity_exhaustion_preserves_open_phase() {
    let gate = LifecycleGate::new();
    gate.state.lock().unwrap().generation = u64::MAX;
    assert!(matches!(
        gate.begin_quiescence(),
        Err(LifecycleError::Exhausted)
    ));
    assert_eq!(gate.phase(), LifecyclePhase::Open);
    assert!(gate.acquire().is_ok());
}

#[test]
fn admitted_permit_remembers_quiescence_after_reopen() {
    let gate = LifecycleGate::new();
    let old = gate.acquire().unwrap();
    assert!(!old.was_quiesced());
    drop(gate.begin_quiescence().unwrap());
    assert_eq!(gate.phase(), LifecyclePhase::Open);
    assert!(old.was_quiesced());
    let next = gate.acquire().unwrap();
    assert!(!next.was_quiesced());
}
