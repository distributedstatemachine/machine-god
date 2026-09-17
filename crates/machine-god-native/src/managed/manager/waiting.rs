//! One shared deadline plus scheduler-owned released/reacquired dependency quota.
use super::super::scheduler::{RunRef, SchedulerError};
use super::{
    Arc, BoxFuture, Context, Instant, JournalMutation, JournalSnapshot, ManagedMailboxJob,
    ManagedManager, ManagedRuntimeError, Poll, command,
};
use machine_god_core::{ManagedFailureCode, ManagedInspect};
use std::{future::poll_fn, sync::Mutex, task::Waker, time::Duration};

#[derive(Clone, Copy)]
enum WaitOutcome {
    Finished(bool),
    Retarget,
}
#[derive(Default)]
struct Signal(Mutex<(Option<WaitOutcome>, Option<Waker>)>);
impl Signal {
    fn finish(&self, outcome: WaitOutcome) {
        let wake = {
            let mut state = self.0.lock().unwrap();
            if state.0.is_some() {
                return;
            }
            state.0 = Some(outcome);
            state.1.take()
        };
        if let Some(wake) = wake {
            wake.wake();
        }
    }
    async fn wait(self: Arc<Self>) -> WaitOutcome {
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
    target: Option<RunRef>,
    handoff: Option<RunRef>,
    future: Option<BoxFuture<'static, Result<WaitOutcome, SchedulerError>>>,
}
impl Drop for Waiter {
    fn drop(&mut self) {
        // An admitted future normally owns cancellation. Preserve that exact
        // custody even in the gap after reacquisition and before the next leg.
        if let Some(source) = self.handoff.take() {
            source.cancel();
        }
    }
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
            target: None,
            handoff: None,
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
                || job.cancellation().is_cancelled()
                || !job.lease().principal().is_live()
                || ((waiter.future.is_none() || job.lease().is_human()) && !job.lease().is_live())
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
                waiter.signal.finish(WaitOutcome::Finished(!settled));
            }
            let target = child.and_then(|child| child.prepared.owner.run());
            if waiter.target.as_ref().is_some_and(|previous| {
                target
                    .as_ref()
                    .is_none_or(|target| !previous.same_run(target))
            }) {
                // The predicate spans the child's FIFO, but a scheduler edge
                // names one actual run. Settle that leg normally: dropping its
                // future here would cancel the original caller instead.
                waiter.signal.finish(WaitOutcome::Retarget);
            }
            if waiter.future.is_none() {
                let signal = waiter.signal.clone();
                if let Some(source) = job.lease().run() {
                    // Registration precedes the first acquisition poll; a
                    // completed predecessor can also remain observable during
                    // successor admission. Neither is a schedulable dependency.
                    if let Some(target) = target.filter(RunRef::is_wait_target) {
                        waiter.target = Some(target.clone());
                        waiter.future = Some(Box::pin(source.dependency_wait(
                            target,
                            signal.wait(),
                            job.cancellation().clone(),
                        )));
                    } else if settled || now >= waiter.deadline {
                        waiter.future = Some(Box::pin(async move { Ok(signal.wait().await) }));
                    }
                } else {
                    let cancellation = job.cancellation().clone();
                    waiter.future = Some(Box::pin(async move {
                        let observed = std::pin::pin!(signal.wait());
                        let cancelled = std::pin::pin!(cancellation.cancelled());
                        match futures_util::future::select(observed, cancelled).await {
                            futures_util::future::Either::Left((value, _)) => Ok(value),
                            futures_util::future::Either::Right(_) => {
                                Err(SchedulerError::Cancelled)
                            }
                        }
                    }));
                }
            }
            if waiter.future.is_some() {
                // The new future is polled below in this same synchronous pass.
                // It resumes scheduler cancellation custody before we yield.
                waiter.handoff.take();
            }
            let result = waiter
                .future
                .as_mut()
                .map(|future| future.as_mut().poll(cx));
            if let Some(Poll::Ready(result)) = result {
                if matches!(result, Ok(WaitOutcome::Retarget)) {
                    // Quota has now been fairly reacquired. Keep the original
                    // job, cancellation and absolute deadline, and validate a
                    // fresh dependency edge on the next shared-manager pass.
                    waiter.handoff = job.lease().run().cloned();
                    waiter.future.take();
                    waiter.target.take();
                    waiter.signal = Arc::default();
                    progress = true;
                    index += 1;
                    continue;
                }
                let mut waiter = self.waiters.remove(index);
                let job = waiter.job.take().unwrap();
                match result {
                    Ok(WaitOutcome::Finished(timeout)) => {
                        self.ready_jobs.push_back((
                            job,
                            timeout,
                            std::mem::take(&mut waiter.operation),
                        ));
                    }
                    Ok(WaitOutcome::Retarget) => unreachable!("retained waiter above"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed::{
        actor::ManagedCommandActor,
        mailbox::ManagedMailboxRequester,
        manager::{factory::PreparedManagedRuntime, tests::Fixture},
    };
    use futures_executor::block_on;
    use machine_god_core::*;
    use machine_god_testkit::{
        InMemorySessionStore, ModelProviderStep, PermissionStep, ScriptedPermissionHandler,
    };
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Forwarder {
        requester: Mutex<Option<ManagedMailboxRequester>>,
        gates: Mutex<BTreeMap<String, tokio::sync::oneshot::Receiver<()>>>,
        started: Mutex<Vec<String>>,
        cancellations: Mutex<BTreeMap<String, CancellationToken>>,
        results: Mutex<Vec<(String, ManagedSubagentResult)>>,
    }
    impl ManagedSubagentAuthority for Forwarder {
        fn execute(
            &self,
            invocation: ManagedSubagentInvocation,
            cancellation: CancellationToken,
        ) -> BoxFuture<'_, Result<ManagedSubagentResult, ManagedSubagentError>> {
            Box::pin(async move {
                let call = invocation.context().call_id.to_string();
                self.started.lock().unwrap().push(call.clone());
                self.cancellations
                    .lock()
                    .unwrap()
                    .insert(call.clone(), cancellation.clone());
                let gate = self.gates.lock().unwrap().remove(&call);
                if let Some(gate) = gate {
                    gate.await.map_err(|_| ManagedSubagentError::Cancelled)?;
                }
                let requester = self.requester.lock().unwrap().clone().unwrap();
                let result = requester.execute(invocation, cancellation).await?;
                self.results.lock().unwrap().push((call, result.clone()));
                Ok(result)
            })
        }
    }
    fn arguments(command: serde_json::Value) -> serde_json::Value {
        serde_json::Value::Object(serde_json::Map::from_iter([("command".into(), command)]))
    }
    fn tool(call: &str, command: serde_json::Value) -> ModelProviderStep {
        ModelProviderStep::events([
            ModelEvent::ToolCall {
                call: ToolCall {
                    id: ToolCallId::new(call).unwrap(),
                    name: ToolName::new("subagent").unwrap(),
                    arguments: arguments(command),
                },
            },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ])
    }
    fn completed() -> ModelProviderStep {
        ModelProviderStep::events([ModelEvent::Stop {
            reason: StopReason::Completed,
        }])
    }
    fn human(f: &mut Fixture, owner: &PreparedManagedRuntime, command: serde_json::Value) {
        let command = ManagedSubagentCommand::decode(arguments(command)).unwrap();
        let mut response = f
            .manager
            .mailbox
            .request_human(
                command,
                &[],
                || {
                    ManagedCommandActor::human(
                        &owner.runtime,
                        owner.owner.principal().clone(),
                        CancellationToken::new(),
                    )
                },
                CancellationToken::new(),
            )
            .unwrap();
        let result = block_on(poll_fn(|cx| {
            if let Poll::Ready(result) = response.as_mut().poll(cx) {
                return Poll::Ready(result.unwrap());
            }
            let progress = f.manager.poll_progress(cx, 100);
            assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
            if progress.is_ready() {
                cx.waker().wake_by_ref();
            }
            Poll::Pending
        }));
        assert!(result.ok, "{result:?}");
    }

    #[test]
    fn fifo_successor_wait_rechecks_dependency_cycle() {
        fifo_successor_wait(false);
    }

    #[test]
    fn cancellation_between_fifo_dependency_legs_cancels_original_turn() {
        fifo_successor_wait(true);
    }

    #[allow(clippy::too_many_lines)] // Actual authorized calls span two FIFO runs and relationship changes.
    fn fifo_successor_wait(cancel_between_legs: bool) {
        let forwarder = Arc::new(Forwarder::default());
        let (a_sender, a_receiver) = tokio::sync::oneshot::channel();
        let (b_sender, b_receiver) = tokio::sync::oneshot::channel();
        forwarder
            .gates
            .lock()
            .unwrap()
            .extend([("a-wait".into(), a_receiver), ("b-head".into(), b_receiver)]);
        let wait = |id| serde_json::json!({"inspect":{"id":id,"sections":["status"],"wait":{"until":"settled","timeout_ms":60000}}});
        let mut f = Fixture::with_engine_setup(
            vec![
                tool(
                    "create-b",
                    serde_json::json!({"create":{"name":"B","mode":"persistent"}}),
                ),
                tool("a-wait", wait("child-2")),
                tool(
                    "b-head",
                    serde_json::json!({"inspect":{"id":"child-2","sections":["status"]}}),
                ),
                completed(),
                tool("b-successor-wait", wait("child-1")),
                completed(),
                completed(),
            ],
            InMemorySessionStore::default(),
            |engine| {
                engine
                    .tool(SubagentTool::shared_authority(forwarder.clone()))
                    .permission_handler(ScriptedPermissionHandler::new((0..8).map(|_| {
                        PermissionStep::Decision(PermissionDecision::Allow {
                            scope: PermissionGrantScope::Once,
                        })
                    })))
            },
        );
        *forwarder.requester.lock().unwrap() = Some(f.requester.clone());
        assert!(f.command(serde_json::json!({"create":{"name":"A","mode":"persistent","prompt":"create B then wait"}})).ok);
        f.drive(|_| {
            forwarder
                .started
                .lock()
                .unwrap()
                .iter()
                .any(|call| call == "a-wait")
        });

        // A remains B's immutable controller. Human relationship edits make B
        // A's current parent, authorizing both inspection directions without a
        // relationship cycle or a fabricated model-call witness.
        let owner = f.notified_foreground(f.notice_session());
        human(
            &mut f,
            &owner,
            serde_json::json!({"relationship":{"id":"child-2","action":"detach"}}),
        );
        human(
            &mut f,
            &owner,
            serde_json::json!({"relationship":{"id":"child-1","action":"reparent","parent_id":"child-2"}}),
        );
        human(
            &mut f,
            &owner,
            serde_json::json!({"message":{"send":{"id":"child-2","content":"first"}}}),
        );
        f.drive(|_| {
            forwarder
                .started
                .lock()
                .unwrap()
                .iter()
                .any(|call| call == "b-head")
        });
        human(
            &mut f,
            &owner,
            serde_json::json!({"message":{"send":{"id":"child-2","content":"successor"}}}),
        );
        a_sender.send(()).unwrap();
        f.drive(|f| f.manager.waiters.len() == 1 && f.manager.waiters[0].future.is_some());
        let source = f
            .manager
            .children
            .iter()
            .find(|c| c.snapshot.head.id == "child-1")
            .unwrap()
            .prepared
            .owner
            .run()
            .unwrap();
        assert!(
            !source.is_executing(),
            "original wait releases execution quota"
        );
        if cancel_between_legs {
            f.manager.limits.work_per_poll = 1;
        }
        b_sender.send(()).unwrap();
        if cancel_between_legs {
            f.drive(|f| {
                f.manager
                    .waiters
                    .iter()
                    .any(|waiter| waiter.handoff.is_some())
            });
            assert!(
                source.is_executing(),
                "the original run fairly reacquired quota"
            );
            forwarder.cancellations.lock().unwrap()["a-wait"].cancel();
            assert!(
                f.manager
                    .pump_waiters(&mut Context::from_waker(Waker::noop()))
            );
            assert!(
                !source.is_executing(),
                "abandonment cancels the original run even between legs"
            );
            return;
        }
        f.drive(|_| {
            forwarder
                .started
                .lock()
                .unwrap()
                .iter()
                .any(|call| call == "b-successor-wait")
        });

        // The fixture clock never advances: rejection must come from cycle
        // validation, not the original wait's timeout. Once both calls are
        // admitted the pure scheduler must either reject or visibly deadlock.
        f.drive(|f| {
            f.manager.waiters.len() == 2
                || forwarder
                    .results
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(_, r)| r.error_code == Some(ManagedFailureCode::DependencyCycle))
        });
        for _ in 0..32 {
            let progress = f
                .manager
                .poll_progress(&mut Context::from_waker(Waker::noop()), 100);
            assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
        }
        assert!(
            f.manager.waiters.len() < 2,
            "FIFO successor cycle was not rejected"
        );
        f.drive(|f| f.manager.waiters.is_empty() && f.manager.children.iter().all(|c| !c.busy()));
        let results = forwarder.results.lock().unwrap();
        let waits: Vec<_> = results
            .iter()
            .filter(|(call, _)| call.ends_with("wait"))
            .collect();
        assert_eq!(waits.len(), 2);
        assert_eq!(waits.iter().filter(|(_, result)| result.ok).count(), 1);
        assert_eq!(waits.iter().filter(|(_, result)| result.error_code == Some(ManagedFailureCode::DependencyCycle)).count(), 1);
    }
}
