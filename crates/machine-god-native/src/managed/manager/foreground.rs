//! Foreground resource custody shares the outer driver, not the display lifetime.
use super::{
    Arc, Context, ManagedManager, ManagedRuntimeError, NativeConversationRuntime, Poll,
    PreparedManagedRuntime, RunRef, RunSettlement, Weak, fmt,
};

struct Identity;

/// Allocation-bound observation/retirement handle, never model-call authority.
#[derive(Clone)]
pub(crate) struct ManagedForegroundSelection(Weak<Identity>);
impl fmt::Debug for ManagedForegroundSelection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ManagedForegroundSelection(..)")
    }
}

pub(super) struct Foreground {
    prepared: PreparedManagedRuntime,
    identity: Arc<Identity>,
    settlement: Option<(RunRef, RunSettlement)>,
    closing: bool,
    closed: bool,
    admission_waiting: bool,
    notice_drain: Option<crate::conversation_runtime::NativeNoticeDrain>,
}
impl Foreground {
    fn matches(&self, selected: &ManagedForegroundSelection) -> bool {
        selected.0.ptr_eq(&Arc::downgrade(&self.identity))
    }
    pub(super) fn executing(&self) -> bool {
        self.prepared
            .owner
            .run()
            .is_some_and(|run| run.is_executing())
    }
    pub(super) fn settling(&self) -> bool {
        self.closing || self.settlement.is_some()
    }
    pub(super) fn close(&mut self) {
        if self.closing {
            return;
        }
        self.closing = true;
        self.prepared.owner.retire();
        let _ = self.prepared.runtime.request_active_cancel();
        let _ = self.prepared.runtime.clear_queued();
        self.prepared.resources.begin_close();
    }
}
impl Drop for Foreground {
    fn drop(&mut self) {
        self.close();
        if let Some(context) = &self.prepared.notice_context {
            context.retire();
        }
    }
}

impl ManagedManager {
    /// The unique manager resolves its own allocation-bound foreground before
    /// capturing a native-human actor. Labels and caller-supplied IDs do not bind it.
    pub(crate) fn request_human_command(
        &mut self,
        selection: &ManagedForegroundSelection,
        command: machine_god_core::ManagedSubagentCommand,
        cancellation: machine_god_core::CancellationToken,
    ) -> Result<super::super::mailbox::ManagedCommandResponse, machine_god_core::ManagedSubagentError>
    {
        if self.closing {
            return Err(machine_god_core::ManagedSubagentError::Unavailable);
        }
        let parent = self
            .foregrounds
            .iter()
            .find(|parent| !parent.closing && !parent.closed && parent.matches(selection))
            .ok_or(machine_god_core::ManagedSubagentError::Unavailable)?;
        let selected_cancel = cancellation.clone();
        self.mailbox.request_human(
            command,
            || {
                super::super::actor::ManagedCommandActor::human(
                    &parent.prepared.runtime,
                    parent.prepared.owner.principal().clone(),
                    selected_cancel,
                )
            },
            cancellation,
        )
    }

    /// Transfer an actually prepared foreground into outer custody. Failure
    /// returns its original owner so the caller can settle it without leaking it.
    pub(crate) fn enroll_foreground(
        &mut self,
        prepared: Box<PreparedManagedRuntime>,
        reservation: &super::ManagedForegroundReservation,
    ) -> Result<super::ManagedForegroundSelection, (ManagedRuntimeError, Box<PreparedManagedRuntime>)>
    {
        let validate = || {
            if self.closing || !prepared.owner.principal().is_live() {
                return Err(ManagedRuntimeError::Unavailable);
            }
            self.validate_foreground_reservation(reservation)?;
            if prepared.runtime.status().active
                || self.foregrounds.iter().any(|parent| {
                    Arc::ptr_eq(
                        parent.prepared.owner.principal(),
                        prepared.owner.principal(),
                    ) || Arc::ptr_eq(&parent.prepared.runtime, &prepared.runtime)
                })
            {
                return Err(ManagedRuntimeError::Invalid);
            }
            Ok(())
        };
        if let Err(error) = validate() {
            return Err((error, prepared));
        }
        if let Some(context) = &prepared.notice_context
            && let Err(error) = self.register_parent_context(context)
        {
            return Err((error, prepared));
        }
        let identity = Arc::new(Identity);
        if let Err(error) = self.consume_foreground_reservation(reservation) {
            return Err((error, prepared));
        }
        let selection = ManagedForegroundSelection(Arc::downgrade(&identity));
        self.remember_principal(prepared.owner.principal());
        self.foregrounds.push(Foreground {
            prepared: *prepared,
            identity,
            settlement: None,
            closing: false,
            closed: false,
            admission_waiting: false,
            notice_drain: None,
        });
        Ok(selection)
    }

    pub(crate) fn foreground_runtime(
        &self,
        selection: &ManagedForegroundSelection,
    ) -> Option<&Arc<NativeConversationRuntime>> {
        self.foregrounds
            .iter()
            .find(|parent| !parent.closing && parent.matches(selection))
            .map(|parent| &parent.prepared.runtime)
    }

    /// Fence this exact foreground while preserving source-ACK cleanup access.
    /// The returned guard remains the only owner allowed to retire/reopen it.
    pub(crate) fn quiesce_foreground(
        &mut self,
        selection: &ManagedForegroundSelection,
    ) -> Result<crate::NativeRuntimeQuiescence, crate::NativeConversationRuntimeError> {
        let parent = self
            .foregrounds
            .iter_mut()
            .find(|parent| parent.matches(selection))
            .ok_or(crate::NativeConversationRuntimeError::Retired)?;
        let guard = parent.prepared.runtime.begin_quiescence()?;
        parent.notice_drain = Some(guard.notice_drain());
        Ok(guard)
    }

    pub(crate) fn foreground_mcp_controls(
        &self,
        selection: &ManagedForegroundSelection,
    ) -> Option<super::factory::ManagedMcpControls> {
        self.foregrounds
            .iter()
            .find(|parent| !parent.closing && parent.matches(selection))?
            .prepared
            .resources
            .mcp_controls()
    }

    pub(crate) fn foreground_turn_settled(&self, selection: &ManagedForegroundSelection) -> bool {
        self.foregrounds
            .iter()
            .find(|parent| parent.matches(selection))
            .map_or_else(
                || selection.0.strong_count() == 0,
                |parent| {
                    !parent.prepared.runtime.status().active
                        && !parent.admission_waiting
                        && parent.settlement.is_none()
                        && parent.prepared.owner.execution_is_idle()
                },
            )
    }

    /// Retires this exact foreground only. Child FIFOs and sibling registrations
    /// remain live; repeated retirement cannot address a replacement allocation.
    pub(crate) fn retire_foreground(&mut self, selection: &ManagedForegroundSelection) -> bool {
        let Some(parent) = self
            .foregrounds
            .iter_mut()
            .find(|parent| parent.matches(selection))
        else {
            return false;
        };
        parent.close();
        true
    }

    pub(super) fn poll_foregrounds(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Result<bool, ManagedRuntimeError> {
        let mut progress = false;
        let mut index = 0;
        while index < self.foregrounds.len() {
            let parent = &mut self.foregrounds[index];
            // Closing MCP startup can be required to settle the original
            // admission cohort; drive it concurrently, not after that receipt.
            if parent.closing
                && !parent.closed
                && let Poll::Ready(result) = parent.prepared.resources.poll_closed(cx)
            {
                result?;
                parent.closed = true;
                progress = true;
            }
            // The host owns and co-polls the actual foreground admission/stream.
            // An active stream or pre-turn admission is not a cleanup receipt.
            if parent.prepared.runtime.status().active {
                index += 1;
                continue;
            }
            match parent.prepared.resources.poll_admission_settled(cx) {
                Poll::Pending => {
                    parent.admission_waiting = true;
                    index += 1;
                    continue;
                }
                Poll::Ready(result) => {
                    result?;
                    progress |= std::mem::take(&mut parent.admission_waiting);
                }
            }
            if parent.settlement.is_none() {
                parent.settlement = parent.prepared.owner.take_settlement();
            }
            if let Some((run, _)) = &parent.settlement
                && let Poll::Ready(result) = parent.prepared.resources.poll_turn_settled(cx, run)
            {
                result?;
                parent
                    .settlement
                    .take()
                    .expect("observed settlement")
                    .1
                    .complete()
                    .map_err(|_| ManagedRuntimeError::Invalid)?;
                progress = true;
            }
            if parent.closing && parent.settlement.is_none() {
                // Do not retire the exact notice context before source ACK and
                // outbox removal have settled in this same outer manager.
                if !parent.prepared.runtime.notice_cleanup_pending() && parent.closed {
                    self.foregrounds.remove(index);
                    self.retry.retry_capacity();
                    progress = true;
                    continue;
                }
            }
            index += 1;
        }
        Ok(progress)
    }
}

pub(super) fn runtime_for_notice(
    foregrounds: &[Foreground],
    context: &Weak<super::super::prompt_context::ParentNoticeContext>,
) -> Option<super::delivery::ClearTarget> {
    foregrounds
        .iter()
        .find(|parent| {
            !parent.prepared.runtime.status().active
                && parent
                    .prepared
                    .notice_context
                    .as_ref()
                    .is_some_and(|selected| context.ptr_eq(&Arc::downgrade(selected)))
        })
        .map(|parent| super::delivery::ClearTarget {
            runtime: parent.prepared.runtime.clone(),
            drain: parent.notice_drain.clone(),
        })
}
