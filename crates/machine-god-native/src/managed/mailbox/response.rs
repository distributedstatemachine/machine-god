use super::{Error, ManagedMailboxWake, ManagedSubagentResult, Reservation};
use machine_god_core::{MAX_SUBAGENT_OUTPUT_BYTES, ManagedRequested};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Waker},
};

type Result = std::result::Result<ManagedSubagentResult, Error>;
pub(super) struct Reply {
    consent_lifetime: Weak<AtomicBool>,
    state: Mutex<State>,
    reservation: Arc<Reservation>,
    wake: ManagedMailboxWake,
}
struct State {
    observer: bool,
    settled: bool,
    result: Option<Result>,
    waker: Option<Waker>,
}
impl Reply {
    pub(super) fn new(
        reservation: Arc<Reservation>,
        wake: ManagedMailboxWake,
        consent_lifetime: Weak<AtomicBool>,
    ) -> Self {
        Self {
            consent_lifetime,
            state: Mutex::new(State {
                observer: true,
                settled: false,
                result: None,
                waker: None,
            }),
            reservation,
            wake,
        }
    }
    pub(super) fn observer_gone(&self) -> bool {
        !self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .observer
    }
    pub(super) fn finish(&self, result: Result) {
        self.retire_consent();
        let mut incoming = Some(result);
        let waker = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.settled {
                return;
            }
            state.settled = true;
            if state.observer {
                state.result = incoming.take();
            }
            state.waker.take()
        };
        drop(incoming);
        if let Some(waker) = waker {
            waker.wake();
        }
    }
    fn retire_consent(&self) {
        if let Some(live) = self.consent_lifetime.upgrade() {
            live.store(false, Ordering::Release);
        }
    }
}

pub(super) struct Response(Option<Arc<Reply>>);
impl Response {
    pub(super) fn new(reply: Arc<Reply>) -> Self {
        Self(Some(reply))
    }
}
impl Future for Response {
    type Output = Result;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result> {
        let mut waker = Some(cx.waker().clone());
        let reply = self.0.as_ref().expect("response polled after completion");
        let (result, old) = {
            let mut state = reply
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let result = state.result.take();
            let old = if result.is_some() {
                state.observer = false;
                state.waker.take()
            } else {
                state.waker.replace(waker.take().expect("new waker"))
            };
            (result, old)
        };
        drop(old);
        drop(waker);
        if let Some(result) = result {
            drop(self.0.take());
            Poll::Ready(result)
        } else {
            Poll::Pending
        }
    }
}
impl Drop for Response {
    fn drop(&mut self) {
        if let Some(reply) = self.0.take() {
            reply.retire_consent();
            let (result, waker) = {
                let mut state = reply
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state.observer = false;
                (state.result.take(), state.waker.take())
            };
            drop(result);
            drop(waker);
            reply.wake.wake();
        }
    }
}
impl Drop for Reply {
    fn drop(&mut self) {
        // Explicit order: payload and callback disappear before its byte lease.
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        drop(state.result.take());
        drop(state.waker.take());
        let _ = &self.reservation;
    }
}

pub(super) fn bounded_result(result: Result) -> Result {
    let result = result?;
    result.validate()?;
    if let Some(ManagedRequested::Inspection(inspection)) = &result.requested
        && let Some(config) = &inspection.configuration
        && (config.notifications.milestones.len() > 32
            || config.notifications.stop_conditions.len() > 8)
    {
        return Err(Error::ResourceLimit);
    }
    // Canonical ownership removes caller-controlled spare String/Vec capacity.
    // Core's counting validation above runs before this bounded allocation.
    let bytes = serde_json::to_vec(&result).map_err(|_| Error::Failed)?;
    if bytes.len() > MAX_SUBAGENT_OUTPUT_BYTES {
        return Err(Error::ResourceLimit);
    }
    drop(result);
    serde_json::from_slice(&bytes).map_err(|_| Error::Failed)
}
