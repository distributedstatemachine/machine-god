//! One shared deadline plus scheduler-owned released/reacquired dependency quota.
use super::super::scheduler::SchedulerError;
use super::{
    Arc, BoxFuture, Context, Instant, JournalMutation, JournalSnapshot, ManagedMailboxJob,
    ManagedManager, ManagedRuntimeError, Poll, command,
};
use machine_god_core::{ManagedFailureCode, ManagedInspect};
use std::{future::poll_fn, sync::Mutex, task::Waker, time::Duration};

#[derive(Default)]
struct Signal(Mutex<(Option<bool>, Option<Waker>)>);
impl Signal {
    fn finish(&self, timed_out: bool) {
        let wake = {
            let mut state = self.0.lock().unwrap();
            if state.0.is_some() {
                return;
            }
            state.0 = Some(timed_out);
            state.1.take()
        };
        if let Some(wake) = wake {
            wake.wake();
        }
    }
    async fn wait(self: Arc<Self>) -> bool {
        poll_fn(|cx| {
            let wake = cx.waker().clone();
            let (result, old) = {
                let mut state = self.0.lock().unwrap();
                (state.0, state.1.replace(wake))
            };
            drop(old);
            result.map_or(Poll::Pending, Poll::Ready)
        })
        .await
    }
}
pub(super) struct Waiter {
    job: Option<ManagedMailboxJob>,
    query: ManagedInspect,
    operation: String,
    deadline: Instant,
    signal: Arc<Signal>,
    future: Option<BoxFuture<'static, Result<bool, SchedulerError>>>,
}
pub(super) struct Approval {
    pub job: Option<ManagedMailboxJob>,
    pub snapshot: JournalSnapshot,
    pub mutation: JournalMutation,
    pub operation: String,
    pub future: BoxFuture<'static, Result<bool, ManagedRuntimeError>>,
    pub approved: bool,
}
impl ManagedManager {
    pub(super) fn add_waiter(
        &mut self,
        job: ManagedMailboxJob,
        query: ManagedInspect,
        operation: String,
    ) {
        if self.waiters.len() == self.limits.waiters {
            job.complete(Ok(command::rejected(
                &operation,
                ManagedFailureCode::ResourceLimit,
            )));
            return;
        }
        let Some(deadline) = self.clock.now().checked_add(Duration::from_millis(
            query.wait.as_ref().expect("wait query").timeout_ms,
        )) else {
            job.complete(Ok(command::rejected(
                &operation,
                ManagedFailureCode::InvalidInspectWait,
            )));
            return;
        };
        self.waiters.push(Waiter {
            job: Some(job),
            query,
            operation,
            deadline,
            signal: Arc::default(),
            future: None,
        });
    }
    #[allow(clippy::too_many_lines)] // Polling preserves quota reacquisition before response observation.
    pub(super) fn pump_waiters(&mut self, cx: &mut Context<'_>) -> bool {
        let now = self.clock.now();
        let mut progress = false;
        let mut approval_index = 0;
        while approval_index < self.approvals.len() {
            let approval = &mut self.approvals[approval_index];
            let job = approval.job.as_ref().unwrap();
            let rejected =
                self.closing || !job.lease().is_live() || job.cancellation().is_cancelled();
            let answer = if rejected {
                Poll::Ready(Ok(false))
            } else if approval.approved {
                Poll::Pending
            } else {
                approval.future.as_mut().poll(cx)
            };
            match answer {
                Poll::Ready(Ok(true)) => {
                    approval.approved = true;
                    progress = true;
                    approval_index += 1;
                }
                Poll::Ready(_) => {
                    let mut approval = self.approvals.remove(approval_index);
                    approval.job.take().unwrap().complete(Ok(command::rejected(
                        &approval.operation,
                        ManagedFailureCode::RelationshipAuthorizationRequired,
                    )));
                    progress = true;
                }
                Poll::Pending => approval_index += 1,
            }
        }
        let mut index = 0;
        while index < self.waiters.len() {
            let waiter = &mut self.waiters[index];
            let job = waiter.job.as_ref().expect("owned waiter job");
            let child = self
                .children
                .iter()
                .find(|child| child.snapshot.head.id == waiter.query.id);
            let settled = child.is_some_and(|child| {
                waiter
                    .query
                    .wait
                    .as_ref()
                    .unwrap()
                    .satisfied(child.snapshot.head.generation, child.snapshot.head.status)
                    && !child.busy()
            });
            if self.closing
                || job.observer_gone()
                || !job.lease().principal().is_live()
                || (waiter.future.is_none() && !job.lease().is_live())
            {
                let mut waiter = self.waiters.remove(index);
                waiter.job.take().unwrap().complete(Ok(command::rejected(
                    &waiter.operation,
                    ManagedFailureCode::CallerUnavailable,
                )));
                progress = true;
                continue;
            }
            if now >= waiter.deadline || settled {
                waiter.signal.finish(!settled);
            }
            if waiter.future.is_none() {
                let signal = waiter.signal.clone();
                if let Some(source) = job.lease().run() {
                    if let Some(target) = child.and_then(|child| child.prepared.owner.run()) {
                        waiter.future = Some(Box::pin(source.dependency_wait(
                            target,
                            signal.wait(),
                            job.cancellation().clone(),
                        )));
                    } else if settled || now >= waiter.deadline {
                        waiter.future = Some(Box::pin(async move { Ok(signal.wait().await) }));
                    }
                } else {
                    waiter.future = Some(Box::pin(async move { Ok(signal.wait().await) }));
                }
            }
            let result = waiter
                .future
                .as_mut()
                .map(|future| future.as_mut().poll(cx));
            if let Some(Poll::Ready(result)) = result {
                let mut waiter = self.waiters.remove(index);
                let job = waiter.job.take().unwrap();
                match result {
                    Ok(timeout) => self.ready_jobs.push_back((job, timeout, waiter.operation)),
                    Err(error) => {
                        let code = if error == SchedulerError::DependencyCycle {
                            ManagedFailureCode::DependencyCycle
                        } else {
                            ManagedFailureCode::DependencyUnavailable
                        };
                        job.complete(Ok(command::rejected(&waiter.operation, code)));
                    }
                }
                progress = true;
            } else {
                index += 1;
            }
        }
        let earliest = self
            .waiters
            .iter()
            .map(|waiter| waiter.deadline)
            .filter(|deadline| *deadline > now)
            .min();
        if self.wait_sleep.as_ref().map(|(deadline, _)| *deadline) != earliest {
            self.wait_sleep.take();
            if let Some(deadline) = earliest {
                let clock = self.clock.clone();
                self.wait_sleep = Some((
                    deadline,
                    Box::pin(async move { clock.sleep_until(deadline).await }),
                ));
            }
        }
        if self
            .wait_sleep
            .as_mut()
            .is_some_and(|(_, future)| future.as_mut().poll(cx).is_ready())
        {
            self.wait_sleep.take();
            cx.waker().wake_by_ref();
        }
        progress
    }
}
