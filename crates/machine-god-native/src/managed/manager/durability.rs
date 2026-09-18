use super::super::store::{
    JournalCreate, JournalError, JournalMutation, JournalPublication, JournalSnapshot,
};
use super::{ManagedJournal, ManagerBlock};
use machine_god_core::BoxFuture;
use std::{
    future::poll_fn,
    sync::{Arc, Mutex},
    task::{Poll, Waker},
};

#[cfg(test)]
mod retry_tests {
    use super::*;
    use std::{
        future::Future,
        task::{Context, Waker},
    };

    #[test]
    fn retry_classes_and_same_class_waiters_do_not_hide_each_other() {
        let gate = RetryGate::default();
        let mut cx = Context::from_waker(Waker::noop());
        let mut capacity = Box::pin(gate.blocked(ManagerBlock::Capacity));
        let mut first = Box::pin(gate.blocked(ManagerBlock::NoticeClear));
        let mut second = Box::pin(gate.blocked(ManagerBlock::NoticeClear));
        let mut manual = Box::pin(gate.blocked(ManagerBlock::Preparation));
        assert!(capacity.as_mut().poll(&mut cx).is_pending());
        assert!(first.as_mut().poll(&mut cx).is_pending());
        assert!(second.as_mut().poll(&mut cx).is_pending());
        assert!(manual.as_mut().poll(&mut cx).is_pending());
        drop(second);
        assert!(gate.recovery_issue().is_some());
        gate.retry_automatic();
        assert!(gate.recovery_issue().is_none());
        assert!(first.as_mut().poll(&mut cx).is_ready());
        assert!(capacity.as_mut().poll(&mut cx).is_pending());
        assert!(manual.as_mut().poll(&mut cx).is_ready());
        gate.retry_capacity();
        assert!(capacity.as_mut().poll(&mut cx).is_ready());
        gate.retry();
        assert!(gate.issue().is_none());
    }

    #[test]
    fn dropping_an_old_generation_cannot_clear_a_new_fence() {
        let gate = RetryGate::default();
        let mut cx = Context::from_waker(Waker::noop());
        let mut old = Box::pin(gate.blocked(ManagerBlock::JournalReceipt));
        assert!(old.as_mut().poll(&mut cx).is_pending());
        gate.retry_automatic();
        let mut new = Box::pin(gate.blocked(ManagerBlock::JournalReceipt));
        assert!(new.as_mut().poll(&mut cx).is_pending());
        drop(old);
        assert!(gate.recovery_issue().is_some());
        drop(new);
        assert!(gate.recovery_issue().is_none());
    }

    #[test]
    fn every_recovery_category_retries_without_releasing_capacity() {
        let gate = RetryGate::default();
        let mut cx = Context::from_waker(Waker::noop());
        let mut capacity = Box::pin(gate.blocked(ManagerBlock::Capacity));
        assert!(capacity.as_mut().poll(&mut cx).is_pending());
        for issue in ManagerBlock::ALL
            .into_iter()
            .filter(|issue| issue.automatic())
        {
            let mut original = Box::pin(gate.blocked(issue));
            assert!(original.as_mut().poll(&mut cx).is_pending());
            assert_eq!(gate.recovery_issue(), Some(issue));
            for _ in 0..32 {
                assert!(original.as_mut().poll(&mut cx).is_pending());
            }
            gate.retry_automatic();
            assert!(original.as_mut().poll(&mut cx).is_ready());
            assert!(capacity.as_mut().poll(&mut cx).is_pending());
        }
        assert!(gate.recovery_issue().is_none());
        assert_eq!(gate.issue(), Some(ManagerBlock::Capacity));
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Failure {
    Rejected(JournalError),
    NotApplied,
    CallerUnavailable,
}
#[derive(Default)]
pub(super) struct RetryGate(Mutex<RetryState>);
#[derive(Default)]
struct RetryState {
    generations: [Arc<()>; 9],
    waiters: [usize; 9],
    waker: Option<Waker>,
}
struct IssueGuard<'a> {
    gate: &'a RetryGate,
    generation: Arc<()>,
    index: usize,
}
impl Drop for IssueGuard<'_> {
    fn drop(&mut self) {
        let waker = {
            let mut state = self.gate.0.lock().unwrap();
            if Arc::ptr_eq(&state.generations[self.index], &self.generation) {
                state.waiters[self.index] -= 1;
                state.waker.take()
            } else {
                None
            }
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}
impl RetryGate {
    pub(super) fn retry(&self) {
        self.retry_matching(|_| true);
    }
    pub(super) fn retry_automatic(&self) {
        self.retry_matching(ManagerBlock::automatic);
    }
    fn retry_matching(&self, matches: impl Fn(ManagerBlock) -> bool) {
        let waker = {
            let mut state = self.0.lock().unwrap();
            for (index, issue) in ManagerBlock::ALL.into_iter().enumerate() {
                if matches(issue) && state.waiters[index] != 0 {
                    state.generations[index] = Arc::new(());
                    state.waiters[index] = 0;
                }
            }
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
    pub(super) fn retry_capacity(&self) {
        self.retry_matching(|issue| issue == ManagerBlock::Capacity);
    }
    pub(super) fn issue(&self) -> Option<ManagerBlock> {
        let state = self.0.lock().unwrap();
        ManagerBlock::ALL
            .into_iter()
            .enumerate()
            .find_map(|(index, issue)| (state.waiters[index] != 0).then_some(issue))
    }
    pub(super) fn recovery_issue(&self) -> Option<ManagerBlock> {
        let state = self.0.lock().unwrap();
        ManagerBlock::ALL
            .into_iter()
            .enumerate()
            .find_map(|(index, issue)| {
                (issue.automatic() && state.waiters[index] != 0).then_some(issue)
            })
    }
    pub(super) async fn blocked(&self, issue: ManagerBlock) {
        let index = ManagerBlock::ALL
            .iter()
            .position(|value| *value == issue)
            .unwrap();
        let generation = {
            let mut state = self.0.lock().unwrap();
            state.waiters[index] += 1;
            state.generations[index].clone()
        };
        let guard = IssueGuard {
            gate: self,
            generation: generation.clone(),
            index,
        };
        poll_fn(|cx| {
            let waker = cx.waker().clone();
            let (ready, old) = {
                let mut state = self.0.lock().unwrap();
                let ready = !Arc::ptr_eq(&state.generations[index], &generation);
                let old = if ready {
                    state.waker.take()
                } else {
                    state.waker.replace(waker)
                };
                (ready, old)
            };
            drop(old);
            if ready {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
        drop(guard);
    }
}
pub(super) fn mutate(
    journal: ManagedJournal,
    gate: Arc<RetryGate>,
    snapshot: JournalSnapshot,
    mutation: JournalMutation,
) -> BoxFuture<'static, Result<JournalSnapshot, Failure>> {
    Box::pin(async move {
        let mutation = loop {
            match super::cancellation::prepare(&journal, &snapshot, &mutation).await {
                Ok(Some(mutation)) => break mutation,
                Ok(None) => break mutation,
                Err(Failure::Rejected(JournalError::Busy | JournalError::Limit)) => {
                    gate.blocked(ManagerBlock::Capacity).await;
                }
                Err(error) => return Err(error),
            }
        };
        confirm(&journal, &gate, None, || {
            journal.mutate(snapshot.clone(), mutation.clone())
        })
        .await
    })
}
pub(super) async fn create(
    journal: &ManagedJournal,
    gate: &RetryGate,
    record: JournalCreate,
    lease: &super::super::actor::ManagedCommandActor,
) -> Result<JournalSnapshot, Failure> {
    confirm(journal, gate, Some(lease), || {
        journal.create(record.clone())
    })
    .await
}

pub(super) async fn mutate_admitted(
    journal: &ManagedJournal,
    gate: &RetryGate,
    snapshot: JournalSnapshot,
    mutation: JournalMutation,
    lease: &super::super::actor::ManagedCommandActor,
) -> Result<JournalSnapshot, Failure> {
    confirm(journal, gate, Some(lease), || {
        journal.mutate(snapshot.clone(), mutation.clone())
    })
    .await
}

pub(super) async fn confirm(
    journal: &ManagedJournal,
    gate: &RetryGate,
    lease: Option<&super::super::actor::ManagedCommandActor>,
    mut operation: impl FnMut() -> BoxFuture<'static, Result<JournalPublication, JournalError>>,
) -> Result<JournalSnapshot, Failure> {
    let receipt = loop {
        if lease.is_some_and(|lease| !lease.is_live()) {
            return Err(Failure::CallerUnavailable);
        }
        match operation().await {
            Ok(JournalPublication::Confirmed(snapshot)) => return Ok(*snapshot),
            Ok(JournalPublication::Ambiguous(receipt)) => break receipt,
            Ok(JournalPublication::NotApplied) => return Err(Failure::NotApplied),
            Err(JournalError::Limit) if lease.is_some() => {
                // This command has not been accepted. Queue, encoded-size and
                // retained-storage limits cannot be repaired by occupying the
                // serialized journal lane: accepted work may need that lane to
                // drain the full FIFO. Return the bounded rejection instead.
                return Err(Failure::Rejected(JournalError::Limit));
            }
            Err(JournalError::Busy | JournalError::Limit) => {
                // Already-accepted internal work has no admitted command lease.
                // Keep its exact settlement custody, including under limits;
                // neither a rejection nor observer loss can undo acceptance.
                gate.blocked(ManagerBlock::Capacity).await;
            }
            Err(error) => return Err(Failure::Rejected(error)),
        }
    };
    loop {
        match journal.reconcile(receipt.clone()).await {
            Ok(JournalPublication::Confirmed(snapshot)) => return Ok(*snapshot),
            Ok(JournalPublication::NotApplied) => return Err(Failure::NotApplied),
            Ok(JournalPublication::Ambiguous(_)) | Err(_) => {
                gate.blocked(ManagerBlock::JournalReceipt).await;
            }
        }
    }
}
