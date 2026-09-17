//! Exact staged notice envelopes are journaled before prompt-context visibility.
use super::super::notices::{
    NoticeError, NoticeHistoryRef, NoticeObservation, NoticePrincipal, NoticeRelationship,
    NoticeTerminal, PreparedNotice, WorkNoticeIdentity,
};
use super::{
    Active, Arc, Child, ChildWrite, Context, JournalMutation, JournalRecord, ManagedAgentState,
    ManagedMailboxJob, ManagedManager, ManagedQueueStatus, ManagedRuntimeError, Poll, WriteAfter,
    command,
};
use machine_god_core::ManagedFailureCode;
use std::{future::Future, num::NonZeroU64, pin::Pin};

impl ManagedManager {
    pub(super) fn retire_child_notice(&mut self, child: &mut Child) {
        if child.snapshot.head.status == ManagedAgentState::Archived {
            self.replay_reset = true;
            let _ = self.notices.retire_source(&NoticePrincipal {
                id: child.snapshot.head.id.clone(),
                generation: NonZeroU64::new(child.snapshot.head.generation).unwrap(),
            });
        }
        if let Some(work) = child.notice.take() {
            if child.snapshot.head.status == ManagedAgentState::Archived {
                let _ = self.notices.close_work(&work);
            } else {
                let _ = self.notices.stop_work(&work);
            }
            self.retained_notices.push(work);
        }
    }
    pub(super) fn refresh_relationship(&self, index: usize) {
        let child = &self.children[index];
        let Some(work) = &child.notice else {
            return;
        };
        let parent = self.notice_parent(index);
        let relationship = NoticeRelationship {
            generation: NonZeroU64::new(child.snapshot.head.revision).unwrap(),
            parent,
            parent_incarnation: child
                .snapshot
                .head
                .parent_owner
                .as_ref()
                .map(|owner| owner.incarnation.clone()),
        };
        let _ = self.notices.set_relationship(work, &relationship);
    }
    fn notice_parent(&self, index: usize) -> Option<NoticePrincipal> {
        // Residency is not a relationship change. Retain the exact admitted
        // target even while its runtime is evicted or its original generation
        // has closed; replay/ACK separately require the historical incarnation.
        let head = &self.children[index].snapshot.head;
        let owner = head.parent_owner.as_ref()?;
        Some(NoticePrincipal {
            id: owner.session_id.to_string(),
            generation: NonZeroU64::new(head.parent_generation?)?,
        })
    }
    pub(super) fn register_notice(&mut self, index: usize) -> Result<bool, ManagedRuntimeError> {
        if self.children[index].notice.is_some() || self.children[index].work.is_none() {
            return Ok(true);
        }
        let child = &self.children[index];
        // The confirmed start revision identifies this durable attempt before
        // skill/readiness work can fail. A notice is not a core-turn witness.
        // Retries of the same work have strictly newer journal revisions.
        let work_generation = child.notice_attempt.ok_or(ManagedRuntimeError::Invalid)?;
        let parent = self.notice_parent(index);
        let identity = WorkNoticeIdentity {
            source: NoticePrincipal {
                id: child.snapshot.head.id.clone(),
                generation: NonZeroU64::new(child.prepared.owner.principal().generation())
                    .ok_or(ManagedRuntimeError::Invalid)?,
            },
            work_id: child.work.as_ref().unwrap().id.clone(),
            work_generation,
        };
        let relationship = NoticeRelationship {
            generation: NonZeroU64::new(child.snapshot.head.revision).unwrap(),
            parent,
            parent_incarnation: child
                .snapshot
                .head
                .parent_owner
                .as_ref()
                .map(|owner| owner.incarnation.clone()),
        };
        let work = self.notices.register_work(
            &identity,
            child
                .work
                .as_ref()
                .unwrap()
                .configuration
                .notifications
                .clone(),
            &relationship,
            child.snapshot.head.notice_cursor,
        );
        match work {
            Ok(work) => {
                self.children[index].notice = Some(work);
                Ok(true)
            }
            Err(NoticeError::Capacity) => Ok(false),
            Err(_) => Err(ManagedRuntimeError::Invalid),
        }
    }
    #[allow(clippy::too_many_lines)] // One ordered staged-publication state transition.
    pub(super) fn pump_notices(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Result<bool, ManagedRuntimeError> {
        self.retained_notices
            .retain(|work| self.notices.release_work(work).is_err());
        if self.active.is_some() {
            return Ok(false);
        }
        let mut progress = false;
        for index in 0..self.children.len() {
            let writing = matches!(&self.active, Some(Active::Child { id, .. } | Active::Load { id, .. }) if id == &self.children[index].snapshot.head.id);
            if writing || !self.children[index].pending.is_empty() {
                continue;
            }
            let child = &mut self.children[index];
            let Some(work) = &child.notice else {
                continue;
            };
            let sequence = NonZeroU64::new(child.snapshot.head.next_sequence)
                .ok_or(ManagedRuntimeError::Invalid)?;
            let history = NoticeHistoryRef {
                record_id: format!("notice-{}", sequence.get()),
                source_sequence: sequence,
            };
            let prepared = if child.notice_started {
                self.notices.prepare_start(work, sequence, Some(&history))
            } else if let Some(status) = child.notice_terminal {
                let outcome = match status {
                    ManagedQueueStatus::Completed => NoticeTerminal::Completed,
                    ManagedQueueStatus::Cancelled => NoticeTerminal::Cancelled,
                    _ => NoticeTerminal::Failed,
                };
                self.notices
                    .prepare_terminal(work, sequence, outcome, Some(&history))
            } else {
                continue;
            };
            match prepared {
                Ok(PreparedNotice::Staged(stage)) => {
                    child.pending.push_back(ChildWrite {
                        mutation: JournalMutation::AppendHistory(vec![JournalRecord::Notice(
                            stage.notice().clone(),
                        )]),
                        after: WriteAfter::Notice {
                            stage: Some(stage),
                            reply: None,
                        },
                    });
                }
                Ok(PreparedNotice::Suppressed) => {
                    // A suppressed occurrence consumed the original source
                    // sequence in memory. Persist a control-only checkpoint
                    // before another occurrence can use the journal sequence;
                    // do not synthesize a visible notice or acknowledgement.
                    child.pending.push_back(ChildWrite {
                        mutation: JournalMutation::SuppressedNotice(sequence.get()),
                        after: WriteAfter::Observe,
                    });
                }
                Ok(PreparedNotice::AlreadyRecorded) | Err(NoticeError::Closed) => {}
                Err(NoticeError::Capacity | NoticeError::Busy) => continue,
                Err(_) => return Err(ManagedRuntimeError::Invalid),
            }
            if child.notice_started {
                child.notice_started = false;
            } else {
                child.notice_terminal = None;
            }
            progress = true;
        }
        if !self.closing {
            if self.deadline.is_none() {
                self.deadline = Some(self.notices.wait_deadline(self.cancellation.clone()));
            }
            if let Some(deadline) = &mut self.deadline {
                match Pin::new(deadline).poll(cx) {
                    Poll::Ready(Ok(())) => {
                        self.deadline.take();
                        for child in &mut self.children {
                            if !child.pending.is_empty()
                                || child.notice_started
                                || child.notice_terminal.is_some()
                            {
                                continue;
                            }
                            let Some(work) = &child.notice else {
                                continue;
                            };
                            let sequence =
                                NonZeroU64::new(child.snapshot.head.next_sequence).unwrap();
                            let observation = NoticeObservation {
                                work: work.clone(),
                                state: child.snapshot.head.status,
                                source_sequence: sequence,
                                history: Some(NoticeHistoryRef {
                                    record_id: format!("notice-{}", sequence.get()),
                                    source_sequence: sequence,
                                }),
                            };
                            if let Ok(PreparedNotice::Staged(stage)) =
                                self.notices.prepare_due(&observation)
                            {
                                child.pending.push_back(ChildWrite {
                                    mutation: JournalMutation::AppendHistory(vec![
                                        JournalRecord::Notice(stage.notice().clone()),
                                    ]),
                                    after: WriteAfter::Notice {
                                        stage: Some(stage),
                                        reply: None,
                                    },
                                });
                                progress = true;
                            }
                        }
                        // Suppression can remove the earliest deadline without
                        // staging a journal write. Install the next timer/change
                        // subscription now, even when the outer pump will sleep.
                        let mut next = self.notices.wait_deadline(self.cancellation.clone());
                        if let Poll::Ready(Err(_)) = Pin::new(&mut next).poll_rearm(cx) {
                            return Err(ManagedRuntimeError::Invalid);
                        }
                        self.deadline = Some(next);
                    }
                    Poll::Ready(Err(_)) => {
                        self.deadline.take();
                        return Err(ManagedRuntimeError::Invalid);
                    }
                    Poll::Pending => {}
                }
            }
        }
        Ok(progress)
    }
    pub(super) fn milestone(&mut self, job: ManagedMailboxJob, name: &str, operation: String) {
        let index = self.children.iter().position(|child| {
            Arc::ptr_eq(child.prepared.owner.principal(), job.lease().principal())
                && child.work.is_some()
                && child.turn.is_some()
        });
        let Some(index) = index else {
            job.complete(Ok(command::rejected(
                &operation,
                ManagedFailureCode::InvalidMilestoneCaller,
            )));
            return;
        };
        let child = &mut self.children[index];
        if !job.lease().is_live() {
            job.complete(Ok(command::rejected(
                &operation,
                ManagedFailureCode::CallerUnavailable,
            )));
            return;
        }
        let Some(work) = &child.notice else {
            job.complete(Ok(command::rejected(
                &operation,
                ManagedFailureCode::MilestoneRequiresActiveWork,
            )));
            return;
        };
        let sequence = NonZeroU64::new(child.snapshot.head.next_sequence).unwrap();
        let history = NoticeHistoryRef {
            record_id: format!("notice-{}", sequence.get()),
            source_sequence: sequence,
        };
        let milestone = |notice, consume_sequence| JournalMutation::Milestone {
            operation_id: operation.clone(),
            work_id: child.work.as_ref().unwrap().id.clone(),
            name: name.to_owned(),
            notice,
            consume_sequence,
        };
        match self
            .notices
            .prepare_milestone(work, sequence, name, Some(&history))
        {
            Ok(PreparedNotice::Staged(stage)) => child.pending.push_back(ChildWrite {
                mutation: milestone(Some(stage.notice().clone()), true),
                after: WriteAfter::Notice {
                    stage: Some(stage),
                    reply: Some((job, operation)),
                },
            }),
            Ok(PreparedNotice::Suppressed) => child.pending.push_back(ChildWrite {
                mutation: milestone(None, true),
                after: WriteAfter::Notice {
                    stage: None,
                    reply: Some((job, operation)),
                },
            }),
            Ok(PreparedNotice::AlreadyRecorded) => child.pending.push_back(ChildWrite {
                mutation: milestone(None, false),
                after: WriteAfter::Notice {
                    stage: None,
                    reply: Some((job, operation)),
                },
            }),
            Err(error) => job.complete(Ok(command::rejected(
                &operation,
                if error == NoticeError::UndeclaredMilestone {
                    ManagedFailureCode::UndeclaredMilestone
                } else {
                    ManagedFailureCode::ResourceLimit
                },
            ))),
        }
    }
}
