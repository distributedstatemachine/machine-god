//! Shared process-local admission fence; never a persistence receipt.

#[cfg(any(target_os = "linux", target_os = "macos", test))]
use machine_god_core::BoxFuture;
use std::{
    fmt,
    sync::{Arc, Mutex},
    task::Waker,
};
#[cfg(any(target_os = "linux", target_os = "macos", test))]
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

const MAX_PERMITS: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LifecyclePhase {
    #[cfg_attr(
        not(any(target_os = "linux", target_os = "macos", test)),
        allow(
            dead_code,
            reason = "Runtime ownership is constructed only on Linux and macOS."
        )
    )]
    Open,
    #[cfg_attr(
        not(any(target_os = "linux", target_os = "macos", test)),
        allow(
            dead_code,
            reason = "Runtime quiescence is constructed only on Linux and macOS."
        )
    )]
    Quiescing,
    Retired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LifecycleError {
    Quiescing,
    Retired,
    Busy,
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    Exhausted,
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    Stale,
}

struct State {
    phase: LifecyclePhase,
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    generation: u64,
    permits: usize,
    waiter: Option<Waker>,
}

pub(crate) struct LifecycleGate {
    state: Mutex<State>,
}
impl fmt::Debug for LifecycleGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LifecycleGate { .. }")
    }
}
impl LifecycleGate {
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                phase: LifecyclePhase::Open,
                generation: 0,
                permits: 0,
                waiter: None,
            }),
        })
    }
    pub(crate) fn phase(&self) -> LifecyclePhase {
        self.state.lock().expect("lifecycle poisoned").phase
    }
    pub(crate) fn acquire(self: &Arc<Self>) -> Result<LifecyclePermit, LifecycleError> {
        let mut state = self.state.lock().expect("lifecycle poisoned");
        match state.phase {
            LifecyclePhase::Quiescing => return Err(LifecycleError::Quiescing),
            LifecyclePhase::Retired => return Err(LifecycleError::Retired),
            LifecyclePhase::Open => {}
        }
        if state.permits == MAX_PERMITS {
            return Err(LifecycleError::Busy);
        }
        state.permits += 1;
        Ok(LifecyclePermit {
            gate: Arc::clone(self),
            #[cfg(any(target_os = "linux", target_os = "macos", test))]
            generation: state.generation,
        })
    }
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    pub(crate) fn begin_quiescence(
        self: &Arc<Self>,
    ) -> Result<LifecycleQuiescence, LifecycleError> {
        let mut state = self.state.lock().expect("lifecycle poisoned");
        match state.phase {
            LifecyclePhase::Quiescing => return Err(LifecycleError::Quiescing),
            LifecyclePhase::Retired => return Err(LifecycleError::Retired),
            LifecyclePhase::Open => {}
        }
        let generation = state
            .generation
            .checked_add(1)
            .ok_or(LifecycleError::Exhausted)?;
        state.generation = generation;
        state.phase = LifecyclePhase::Quiescing;
        Ok(LifecycleQuiescence {
            gate: Arc::clone(self),
            generation,
            retired: false,
        })
    }
}

pub(crate) struct LifecyclePermit {
    gate: Arc<LifecycleGate>,
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    generation: u64,
}
impl LifecyclePermit {
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    pub(crate) fn belongs_to(&self, gate: &Arc<LifecycleGate>) -> bool {
        Arc::ptr_eq(&self.gate, gate)
    }
    /// A cancelled transition cannot undo its request for an admitted job,
    /// including the interval before runtime publishes that job's handle.
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    pub(crate) fn was_quiesced(&self) -> bool {
        self.gate
            .state
            .lock()
            .expect("lifecycle poisoned")
            .generation
            != self.generation
    }
}
impl Drop for LifecyclePermit {
    fn drop(&mut self) {
        let wake = {
            let mut state = self.gate.state.lock().expect("lifecycle poisoned");
            state.permits -= 1;
            if state.permits == 0 {
                state.waiter.take()
            } else {
                None
            }
        };
        if let Some(wake) = wake {
            wake.wake();
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
pub(crate) struct LifecycleQuiescence {
    gate: Arc<LifecycleGate>,
    generation: u64,
    retired: bool,
}
#[cfg(any(target_os = "linux", target_os = "macos", test))]
impl LifecycleQuiescence {
    pub(crate) fn wait_idle(&mut self) -> BoxFuture<'_, Result<(), LifecycleError>> {
        Box::pin(IdleWait { owner: self })
    }
    pub(crate) fn retire(mut self) -> Result<(), LifecycleError> {
        let wake = {
            let mut state = self.gate.state.lock().expect("lifecycle poisoned");
            if state.phase != LifecyclePhase::Quiescing || state.generation != self.generation {
                return Err(LifecycleError::Stale);
            }
            if state.permits != 0 {
                return Err(LifecycleError::Busy);
            }
            state.phase = LifecyclePhase::Retired;
            self.retired = true;
            state.waiter.take()
        };
        if let Some(wake) = wake {
            wake.wake();
        }
        Ok(())
    }
}
#[cfg(any(target_os = "linux", target_os = "macos", test))]
impl Drop for LifecycleQuiescence {
    fn drop(&mut self) {
        let wake = {
            let mut state = self.gate.state.lock().expect("lifecycle poisoned");
            if self.retired
                || state.phase != LifecyclePhase::Quiescing
                || state.generation != self.generation
            {
                return;
            }
            state.phase = LifecyclePhase::Open;
            state.waiter.take()
        };
        if let Some(wake) = wake {
            wake.wake();
        }
    }
}
#[cfg(any(target_os = "linux", target_os = "macos", test))]
struct IdleWait<'a> {
    owner: &'a mut LifecycleQuiescence,
}
#[cfg(any(target_os = "linux", target_os = "macos", test))]
impl Future for IdleWait<'_> {
    type Output = Result<(), LifecycleError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let new = cx.waker().clone();
        let (result, old) = {
            let mut state = self.owner.gate.state.lock().expect("lifecycle poisoned");
            if state.phase != LifecyclePhase::Quiescing || state.generation != self.owner.generation
            {
                (Poll::Ready(Err(LifecycleError::Stale)), Some(new))
            } else if state.permits == 0 {
                (Poll::Ready(Ok(())), Some(new))
            } else {
                (Poll::Pending, state.waiter.replace(new))
            }
        };
        drop(old);
        result
    }
}
#[cfg(any(target_os = "linux", target_os = "macos", test))]
impl Drop for IdleWait<'_> {
    fn drop(&mut self) {
        let old = {
            let mut state = self.owner.gate.state.lock().expect("lifecycle poisoned");
            if state.generation == self.owner.generation {
                state.waiter.take()
            } else {
                None
            }
        };
        drop(old);
    }
}

#[cfg(test)]
mod tests;
