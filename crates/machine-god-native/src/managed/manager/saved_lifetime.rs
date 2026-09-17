//! Saved-generation cleanup owns one resident reservation outside the command lane.
use super::{
    Active, Arc, BoxFuture, Context, JournalSnapshot, ManagedMailboxJob, ManagedManager,
    ManagerBlock, Poll, Weak, command, delivery, factory::PreparedManagedRuntime,
};
use crate::managed::prompt_context::ParentNoticeContext;
use machine_god_core::ManagedFailureCode;

pub(super) type Preparation =
    BoxFuture<'static, Result<PreparedManagedRuntime, ManagedFailureCode>>;

enum State {
    Preparing(Preparation),
    Restored {
        prepared: PreparedManagedRuntime,
        registered: bool,
        closing: bool,
    },
    Ready,
}

pub(super) struct Pending {
    job: ManagedMailboxJob,
    snapshot: JournalSnapshot,
    operation: String,
    reopen: bool,
    state: State,
    retry: Option<BoxFuture<'static, ()>>,
}
impl Pending {
    pub(super) fn new(
        job: ManagedMailboxJob,
        snapshot: JournalSnapshot,
        operation: String,
        reopen: bool,
        preparation: Preparation,
    ) -> Self {
        Self {
            job,
            snapshot,
            operation,
            reopen,
            state: State::Preparing(preparation),
            retry: None,
        }
    }

    pub(super) fn id(&self) -> &str {
        &self.snapshot.head.id
    }

    pub(super) fn context_reservation(&self) -> usize {
        usize::from(!matches!(&self.state, State::Restored {
            prepared, registered: true, ..
        } if prepared.notice_context.is_some()))
    }

    pub(super) fn refresh_snapshot(&mut self, snapshot: &JournalSnapshot) {
        if self.snapshot.head.id == snapshot.head.id
            && self.snapshot.head.generation == snapshot.head.generation
            && self.snapshot.head.transcript == snapshot.head.transcript
        {
            self.snapshot = snapshot.clone();
        }
    }

    pub(super) fn begin_close(&mut self) {
        if let State::Restored { prepared, .. } = &mut self.state {
            prepared.resources.begin_close();
        }
    }
}

impl ManagedManager {
    #[allow(clippy::too_many_lines)] // Original preparation, delivery and closure remain distinct custody phases.
    pub(super) fn poll_saved_lifetimes(&mut self, cx: &mut Context<'_>, now_ms: i64) -> bool {
        let mut index = 0;
        let mut progress = false;
        while index < self.saved_lifetimes.len() {
            if let Some(retry) = &mut self.saved_lifetimes[index].retry {
                if retry.as_mut().poll(cx).is_pending() {
                    index += 1;
                    continue;
                }
                self.saved_lifetimes[index].retry.take();
            }
            if let State::Preparing(future) = &mut self.saved_lifetimes[index].state {
                let Poll::Ready(result) = future.as_mut().poll(cx) else {
                    index += 1;
                    continue;
                };
                match result {
                    Ok(prepared) if !self.saved_lifetimes[index].reopen => {
                        let pending = self.saved_lifetimes.remove(index);
                        // Close intent already belongs to this exact job. The
                        // ordinary resident archive path owns all subsequent
                        // delivery, journal publication and actual retirement.
                        self.apply_outcome(
                            command::Outcome {
                                job: pending.job,
                                snapshot: Some(pending.snapshot),
                                prepared: Some(prepared),
                                action: command::Action::Archive,
                                replay_changed: true,
                            },
                            pending.operation,
                            now_ms,
                        );
                        progress = true;
                        continue;
                    }
                    Ok(prepared) => {
                        self.saved_lifetimes[index].state = State::Restored {
                            prepared,
                            registered: false,
                            closing: false,
                        };
                    }
                    Err(code) => {
                        let pending = self.saved_lifetimes.remove(index);
                        pending
                            .job
                            .complete(Ok(command::rejected(&pending.operation, code)));
                        self.retry.retry_capacity();
                        progress = true;
                        continue;
                    }
                }
                progress = true;
            }
            let context = match &self.saved_lifetimes[index].state {
                State::Restored {
                    prepared,
                    registered: false,
                    ..
                } => prepared.notice_context.clone(),
                _ => None,
            };
            if let Some(context) = context {
                // This old generation is only a cleanup owner, never a newly
                // active recipient or an excuse to start an idle model turn.
                if self.stage_parent_context(&context).is_err() {
                    let gate = self.retry.clone();
                    self.saved_lifetimes[index].retry = Some(Box::pin(async move {
                        gate.blocked(ManagerBlock::Preparation).await;
                    }));
                    progress = true;
                    index += 1;
                    continue;
                }
            }
            if let State::Restored {
                prepared,
                registered,
                closing,
            } = &mut self.saved_lifetimes[index].state
            {
                *registered = true;
                if prepared.runtime.notice_cleanup_pending() {
                    index += 1;
                    continue;
                }
                if !*closing {
                    prepared.resources.begin_close();
                    *closing = true;
                    progress = true;
                }
                match prepared.resources.poll_closed(cx) {
                    Poll::Pending => {
                        index += 1;
                        continue;
                    }
                    Poll::Ready(Err(_)) => {
                        let gate = self.retry.clone();
                        self.saved_lifetimes[index].retry = Some(Box::pin(async move {
                            gate.blocked(ManagerBlock::Cleanup).await;
                        }));
                        progress = true;
                        index += 1;
                        continue;
                    }
                    Poll::Ready(Ok(())) => {}
                }
                // Drop the old runtime/principal/resources before the new
                // generation can enter its independently scheduled command.
                // The pending entry keeps its resident reservation throughout.
                self.saved_lifetimes[index].state = State::Ready;
                progress = true;
            }
            if self.closing && matches!(self.saved_lifetimes[index].state, State::Ready) {
                let pending = self.saved_lifetimes.remove(index);
                pending.job.complete(Ok(command::rejected(
                    &pending.operation,
                    ManagedFailureCode::CallerUnavailable,
                )));
                self.retry.retry_capacity();
                progress = true;
            } else {
                index += 1;
            }
        }
        progress
    }

    pub(super) fn begin_saved_reopen(&mut self, now_ms: i64) -> bool {
        if self.closing {
            return false;
        }
        let Some(index) = self
            .saved_lifetimes
            .iter()
            .position(|pending| matches!(pending.state, State::Ready))
        else {
            return false;
        };
        let pending = self.saved_lifetimes.remove(index);
        // Removing our own reservation and immediately capturing the command
        // environment transfers that same slot; another admission cannot steal it.
        let environment = self.environment(now_ms, pending.operation.clone(), None);
        self.active = Some(Active::Command {
            target: Some(pending.snapshot.head.id.clone()),
            operation: pending.operation,
            future: Box::pin(command::lifecycle::resume_reopen(
                pending.job,
                environment,
                pending.snapshot,
            )),
        });
        true
    }
}

pub(super) fn runtime_for_notice(
    pending: &[Pending],
    context: &Weak<ParentNoticeContext>,
) -> Option<delivery::ClearTarget> {
    pending.iter().find_map(|pending| {
        let State::Restored { prepared, .. } = &pending.state else {
            return None;
        };
        (!prepared.runtime.status().active
            && prepared
                .notice_context
                .as_ref()
                .is_some_and(|original| context.ptr_eq(&Arc::downgrade(original))))
        .then(|| delivery::ClearTarget {
            runtime: prepared.runtime.clone(),
            drain: None,
        })
    })
}
