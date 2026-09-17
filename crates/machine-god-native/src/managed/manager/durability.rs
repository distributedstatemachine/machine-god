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
    generations: [Arc<()>; 4],
    issue: Option<ManagerBlock>,
    waker: Option<Waker>,
}
impl RetryGate {
    pub(super) fn retry(&self) {
        let waker = {
            let mut state = self.0.lock().unwrap();
            state.generations = std::array::from_fn(|_| Arc::new(()));
            state.issue = None;
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
    pub(super) fn retry_capacity(&self) {
        let wake = {
            let mut state = self.0.lock().unwrap();
            state.generations[0] = Arc::new(());
            if state.issue == Some(ManagerBlock::Capacity) {
                state.issue = None;
            }
            state.waker.take()
        };
        if let Some(wake) = wake {
            wake.wake();
        }
    }
    pub(super) fn issue(&self) -> Option<ManagerBlock> {
        self.0.lock().unwrap().issue
    }
    pub(super) async fn blocked(&self, issue: ManagerBlock) {
        let index = match issue {
            ManagerBlock::Capacity => 0,
            ManagerBlock::Journal => 1,
            ManagerBlock::Preparation => 2,
            ManagerBlock::Cleanup => 3,
        };
        let generation = {
            let mut state = self.0.lock().unwrap();
            state.issue = Some(issue);
            state.generations[index].clone()
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
    }
}
pub(super) fn mutate(
    journal: ManagedJournal,
    gate: Arc<RetryGate>,
    snapshot: JournalSnapshot,
    mutation: JournalMutation,
) -> BoxFuture<'static, Result<JournalSnapshot, Failure>> {
    Box::pin(async move {
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
                gate.blocked(ManagerBlock::Journal).await;
            }
        }
    }
}
