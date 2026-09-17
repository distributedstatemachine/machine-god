use super::{
    Active, Child, ChildWrite, Context, JournalIntent, JournalMutation, JournalRecord, JournalWork,
    ManagedAgentState, ManagedHistoryItem, ManagedHistoryKind, ManagedManager, ManagedOutcome,
    ManagedPermissionMode, ManagedQueueStatus, ManagedRuntimeError, ManagedToolActivity,
    ManagedToolPhase, ModelEvent, Pin, Poll, Stream, TurnEvent, WriteAfter, command, projection,
    target_id,
};

impl ManagedManager {
    #[allow(clippy::too_many_lines)] // Ordering joins independent stream, journal and actual cleanup states.
    pub(super) fn poll_child(
        &mut self,
        index: usize,
        cx: &mut Context<'_>,
        now_ms: i64,
    ) -> Result<bool, ManagedRuntimeError> {
        if self.children[index].work.is_some() && !self.register_notice(index)? {
            return Ok(false);
        }
        let child = &mut self.children[index];
        let writing = match &self.active {
            Some(Active::Child { id, .. } | Active::Load { id, .. }) => {
                id == &child.snapshot.head.id
            }
            Some(Active::Command { target, .. }) => {
                target.as_ref() == Some(&child.snapshot.head.id)
            }
            None
            | Some(
                Active::Delivery(_)
                | Active::Replay(_)
                | Active::Catalog { .. }
                | Active::Observation(_),
            ) => false,
        } || self.pending_job.as_ref().is_some_and(|(job, _, _)| {
            target_id(job.command()) == Some(child.snapshot.head.id.as_str())
        });
        let mut progress = false;
        if child.admission_pending
            && child.starting.is_none()
            && child.turn.is_none()
            && let Poll::Ready(result) = child.prepared.resources.poll_admission_settled(cx)
        {
            result?;
            child.admission_pending = false;
            child.actual_settled = true;
            progress = true;
        }
        // Actual cleanup is independent of a pending journal receipt or frame.
        if child.settlement.is_none() && child.turn.is_none() && child.starting.is_none() {
            child.settlement = child.prepared.owner.take_settlement();
        }
        if let Some((run, _)) = &child.settlement
            && let Poll::Ready(result) = child.prepared.resources.poll_turn_settled(cx, run)
        {
            result?;
            let (_, settlement) = child.settlement.take().unwrap();
            settlement
                .complete()
                .map_err(|_| ManagedRuntimeError::Invalid)?;
            child.actual_settled = true;
            progress = true;
        }
        if writing || !child.pending.is_empty() {
            return Ok(progress);
        }
        if let Some(future) = &mut child.starting {
            match future.as_mut().poll(cx) {
                Poll::Pending => {}
                Poll::Ready(Ok(Some(turn))) => {
                    child.admission_pending = false;
                    child.starting.take();
                    child.turn = Some(turn);
                    if self.closing || child.snapshot.head.intent.is_some() {
                        let _ = child.prepared.runtime.request_active_cancel();
                    }
                    return Ok(true);
                }
                Poll::Ready(_) => {
                    child.starting.take();
                    Self::finish_child(child, ManagedQueueStatus::Failed, self.closing);
                    return Ok(true);
                }
            }
        }
        if let Some(turn) = &mut child.turn {
            match Pin::new(turn).poll_next(cx) {
                Poll::Pending => {}
                Poll::Ready(Some(Ok(event))) => {
                    Self::observe(child, event, now_ms, self.closing);
                    progress = true;
                }
                Poll::Ready(Some(Err(_)) | None) => {
                    child.turn.take();
                    Self::finish_child(child, ManagedQueueStatus::Failed, self.closing);
                    progress = true;
                }
            }
        }
        if self.closing
            && child.work.is_none()
            && child.starting.is_none()
            && child.turn.is_none()
            && !child.snapshot.head.queue.is_empty()
            && matches!(
                child.snapshot.head.status,
                ManagedAgentState::Queued
                    | ManagedAgentState::Running
                    | ManagedAgentState::AwaitingApproval
            )
        {
            child.pending.push_back(ChildWrite {
                mutation: JournalMutation::HeadState {
                    work_id: child.snapshot.head.queue[0].id.clone(),
                    status: ManagedQueueStatus::Interrupted,
                    failure: None,
                },
                after: WriteAfter::Observe,
            });
            progress = true;
        }
        if child.control_requested && !child.busy() && child.actual_settled {
            if child.snapshot.head.intent == Some(JournalIntent::Archive) {
                if child.prepared.runtime.notice_cleanup_pending() {
                    // Exact source ACK and parent clear must settle before this
                    // source/runtime incarnation is retired. A saved outbox or
                    // uncertain publication has custody even without a receipt.
                    return Ok(progress);
                }
                child.pending.push_back(ChildWrite {
                    mutation: JournalMutation::Archive,
                    after: WriteAfter::Archived,
                });
            } else if child.snapshot.head.intent == Some(JournalIntent::Cancel) {
                let mutation =
                    child
                        .snapshot
                        .head
                        .queue
                        .first()
                        .map_or(JournalMutation::CancelIdle, |work| {
                            JournalMutation::HeadState {
                                work_id: work.id.clone(),
                                status: ManagedQueueStatus::Cancelled,
                                failure: None,
                            }
                        });
                child.pending.push_back(ChildWrite {
                    mutation,
                    after: WriteAfter::Observe,
                });
            } else {
                child.control_requested = false;
                if let Some(job) = child.control.take() {
                    let result = command::receipt(
                        child.control_operation.as_deref().unwrap(),
                        &child.snapshot,
                        ManagedOutcome::LifecycleChanged,
                    );
                    child.control_operation.take();
                    job.complete(Ok(result));
                }
            }
            progress = true;
        }
        if (self.closing || child.closing)
            && !child.busy()
            && child.actual_settled
            && !child.prepared.runtime.notice_cleanup_pending()
        {
            if let Some(job) = child.control.take() {
                let result = command::receipt(
                    child.control_operation.as_deref().unwrap(),
                    &child.snapshot,
                    ManagedOutcome::LifecycleChanged,
                );
                job.complete(Ok(result));
            }
            child.prepared.resources.begin_close();
            // Removal is deferred to admit_next so the fair iteration's indices stay stable.
            child.closing = true;
        }
        // Until notice custody settles, leave the child in this normal lane so
        // its original clear can still be driven before retirement.
        Ok(progress)
    }
    pub(super) fn start_child(
        child: &mut Child,
        work: JournalWork,
        now_ms: i64,
    ) -> Result<(), ManagedRuntimeError> {
        let effort = crate::NativeReasoningEffort::parse(
            work.configuration.effort.as_deref().unwrap_or("auto"),
        )
        .map_err(|_| ManagedRuntimeError::Invalid)?;
        let preferences = crate::NativeModelPreferences::new(
            work.configuration
                .model
                .as_deref()
                .unwrap_or(crate::AI_GATEWAY_DEFAULT_MODEL),
            effort,
            false,
        )
        .map_err(|_| ManagedRuntimeError::Invalid)?;
        child
            .prepared
            .runtime
            .set_model_preferences(preferences)
            .map_err(|_| ManagedRuntimeError::Unavailable)?;
        if let Some(permissions) = child.prepared.runtime.permissions() {
            let mode = match work.configuration.permission_mode {
                ManagedPermissionMode::Ask => crate::PermissionMode::Ask,
                ManagedPermissionMode::Auto => crate::PermissionMode::Auto,
                ManagedPermissionMode::Yolo => crate::PermissionMode::Yolo,
            };
            permissions
                .set_mode(mode)
                .map_err(|_| ManagedRuntimeError::Unavailable)?;
        }
        // Keep enqueue synchronous so cancellation cannot race a deferred
        // insertion. Admission errors settle through this work's owned start
        // lane, not as an error that fences the entire manager.
        let queued = if work.skills.is_empty() {
            child.prepared.runtime.enqueue(work.content.clone().into())
        } else if let Some(skills) = &child.prepared.skills {
            child.prepared.runtime.enqueue_with_skill_references(
                work.content.clone().into(),
                skills.catalog.clone(),
                &work.skills,
                skills.workers.clone(),
            )
        } else {
            Err(crate::NativeConversationRuntimeError::Skills(
                crate::NativeSkillsQueueError::WorkerUnavailable,
            ))
        };
        let runtime = child.prepared.runtime.clone();
        child.starting = Some(Box::pin(async move {
            queued?;
            runtime.start_next(now_ms).await
        }));
        child.admission_pending = true;
        child.work = Some(work);
        child.actual_settled = false;
        child.assistant.clear();
        child.assistant_truncated = false;
        Ok(())
    }
    fn observe(
        child: &mut Child,
        event: machine_god_core::EngineEvent,
        now_ms: i64,
        shutdown: bool,
    ) {
        match event.payload {
            TurnEvent::Started => child.notice_started = true,
            TurnEvent::Model {
                event: ModelEvent::TextDelta { text },
            } => {
                let (text, cut) = projection::prefix(
                    &text,
                    (16 * 1024usize).saturating_sub(child.assistant.len()),
                );
                child.assistant.push_str(&text);
                child.assistant_truncated |= cut;
            }
            TurnEvent::PermissionRequested { .. } => {
                Self::transition(child, ManagedQueueStatus::AwaitingApproval);
            }
            TurnEvent::PermissionResolved { .. } => {
                Self::transition(child, ManagedQueueStatus::Running);
            }
            TurnEvent::ToolStarted { call } => {
                if child.tools.len() < 64 {
                    child.tools.push((call.id, call.name.to_string()));
                }
                Self::tool(
                    child,
                    event.sequence,
                    now_ms,
                    call.name.to_string(),
                    ManagedToolPhase::Started,
                );
            }
            TurnEvent::ToolFinished { call_id, output } => {
                if let Some(index) = child.tools.iter().position(|(id, _)| id == &call_id) {
                    let (_, name) = child.tools.remove(index);
                    Self::tool(
                        child,
                        event.sequence,
                        now_ms,
                        name,
                        if output.is_error {
                            ManagedToolPhase::Failed
                        } else {
                            ManagedToolPhase::Succeeded
                        },
                    );
                }
            }
            TurnEvent::Completed { .. } => {
                child.turn.take();
                Self::finish_child(child, ManagedQueueStatus::Completed, shutdown);
            }
            TurnEvent::Failed { .. } => {
                child.turn.take();
                Self::finish_child(child, ManagedQueueStatus::Failed, shutdown);
            }
            _ => {}
        }
    }
    fn transition(child: &mut Child, status: ManagedQueueStatus) {
        if let Some(work) = &child.work
            && child.snapshot.head.intent.is_none()
            && child
                .snapshot
                .head
                .queue
                .first()
                .is_some_and(|first| first.status != status)
        {
            child.pending.push_back(ChildWrite {
                mutation: JournalMutation::HeadState {
                    work_id: work.id.clone(),
                    status,
                    failure: None,
                },
                after: WriteAfter::Observe,
            });
        }
    }
    fn tool(child: &mut Child, sequence: u64, now_ms: i64, name: String, phase: ManagedToolPhase) {
        child.pending.push_back(ChildWrite {
            mutation: JournalMutation::AppendHistory(vec![JournalRecord::Tool(
                ManagedToolActivity {
                    sequence,
                    revision: child.snapshot.head.revision + 1,
                    timestamp_ms: now_ms,
                    work_id: child.work.as_ref().map(|work| work.id.clone()),
                    tool_name: name,
                    phase,
                },
            )]),
            after: WriteAfter::Observe,
        });
    }
    pub(super) fn finish_child(child: &mut Child, status: ManagedQueueStatus, shutdown: bool) {
        let Some(work) = &child.work else {
            return;
        };
        let status = if child.snapshot.head.intent.is_some() {
            ManagedQueueStatus::Cancelled
        } else if shutdown {
            ManagedQueueStatus::Interrupted
        } else {
            status
        };
        if status != ManagedQueueStatus::Interrupted {
            child.notice_terminal = Some(status);
        }
        let (user, truncated) = projection::prefix(&work.content, 16 * 1024);
        child.pending.push_back(ChildWrite {
            mutation: JournalMutation::AppendHistory(vec![JournalRecord::History(
                ManagedHistoryItem {
                    kind: if status == ManagedQueueStatus::Interrupted {
                        ManagedHistoryKind::Interrupted
                    } else {
                        ManagedHistoryKind::Conversation
                    },
                    work_id: Some(work.id.clone()),
                    user: Some(user),
                    assistant: Some(child.assistant.clone()),
                    user_truncated: truncated,
                    assistant_truncated: child.assistant_truncated,
                },
            )]),
            after: WriteAfter::Observe,
        });
        child.pending.push_back(ChildWrite {
            mutation: JournalMutation::HeadState {
                work_id: work.id.clone(),
                status,
                failure: (status == ManagedQueueStatus::Failed).then(|| "child turn failed".into()),
            },
            after: WriteAfter::Terminal,
        });
    }
}
