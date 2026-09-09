//! A bounded native control lane; accepted publications are never select futures.

use super::{
    NativeInteractiveError, NativeInteractiveOutcome, NativeInteractiveSession, transition,
};
use crate::{
    FileUndoError, FileUndoOutcome, NativeConversationRuntime, NativeConversationRuntimeError,
    NativeModelPreferenceCommit, NativeModelPreferencePersistence, NativeModelPreferences,
    NativePermissionRuleProposal, NativeQueuedJobId, NativeSessionMetadata, NativeSessionOrigin,
    NativeUserConfigStore,
};
use machine_god_core::{
    BackgroundOutputOwner, BoxFuture, InferenceOptions, PermissionError, SessionRevision,
};
use std::{
    fmt,
    sync::Arc,
    task::{Context, Poll},
};

/// Explicit native operations. A rule proposal must have been confirmed by the
/// caller's human-confirmation boundary; merely proposing a rule is not consent.
pub enum NativeInteractiveControl {
    UndoLast,
    Rename {
        title: String,
    },
    Compact,
    Continue {
        options: InferenceOptions,
    },
    SaveModelSession,
    SaveModelDefaults {
        store: Arc<NativeUserConfigStore>,
    },
    ConfirmPermissionRule {
        proposal: NativePermissionRuleProposal,
    },
    /// Explicit human slash edits configured patterns, not exact-action grants.
    Allowlist {
        request: crate::NativeAllowlistRequest,
        store: Arc<NativeUserConfigStore>,
    },
}

impl fmt::Debug for NativeInteractiveControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeInteractiveControl { .. }")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeInteractiveControlId(u64);
impl NativeInteractiveControlId {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

pub enum NativeInteractiveControlError {
    Undo(FileUndoError),
    Runtime(NativeConversationRuntimeError),
    Permission(PermissionError),
    Allowlist(crate::NativeAllowlistError),
    Unavailable,
}
impl fmt::Debug for NativeInteractiveControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeInteractiveControlError { .. }")
    }
}
impl fmt::Display for NativeInteractiveControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("native interactive control unavailable")
    }
}
impl std::error::Error for NativeInteractiveControlError {}
impl From<NativeConversationRuntimeError> for NativeInteractiveControlError {
    fn from(error: NativeConversationRuntimeError) -> Self {
        Self::Runtime(error)
    }
}

/// Exact receipts from existing native operations, not all-target success flags.
pub enum NativeInteractiveControlReceipt {
    Undone(FileUndoOutcome),
    Renamed(SessionRevision),
    Compacted(bool),
    Continued(NativeQueuedJobId),
    ModelSession(NativeModelPreferencePersistence),
    ModelDefaults(NativeModelPreferenceCommit),
    PermissionRuleConfirmed(SessionRevision),
    Allowlist(crate::NativeAllowlistReceipt),
}
impl fmt::Debug for NativeInteractiveControlReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeInteractiveControlReceipt { .. }")
    }
}

pub struct NativeInteractiveControlOutcome {
    pub id: NativeInteractiveControlId,
    pub source: BackgroundOutputOwner,
    pub result: Result<NativeInteractiveControlReceipt, NativeInteractiveControlError>,
}
impl fmt::Debug for NativeInteractiveControlOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeInteractiveControlOutcome { .. }")
    }
}
impl NativeInteractiveControlOutcome {
    pub(super) fn failed(&self) -> bool {
        match &self.result {
            Err(_) => true,
            Ok(NativeInteractiveControlReceipt::ModelDefaults(commit)) => {
                commit.session.is_err() || commit.user_defaults.is_err()
            }
            Ok(NativeInteractiveControlReceipt::Allowlist(receipt)) => receipt.failed(),
            Ok(_) => false,
        }
    }
}

pub(super) struct OwnedControl {
    id: NativeInteractiveControlId,
    source: BackgroundOutputOwner,
    pub(super) future:
        BoxFuture<'static, Result<NativeInteractiveControlReceipt, NativeInteractiveControlError>>,
}

impl NativeInteractiveSession {
    /// Parses against the retained host without granting the CLI another host handle.
    /// # Errors
    /// Returns the native grammar/size error without any effects.
    pub fn parse_allowlist(
        &self,
        rest: &str,
    ) -> Result<crate::NativeAllowlistRequest, crate::NativeAllowlistParseError> {
        self.host.parse_allowlist(rest)
    }

    /// Accepts one exact-current-runtime control. Saves remain inert until owner
    /// progress; continuation performs only its existing synchronous queue check.
    ///
    /// # Errors
    /// Rejects closed, switching or occupied control/outcome lanes, invalid title,
    /// exhausted identity, and the native continuation admission/input bounds.
    /// # Panics
    /// Panics if native runtime state was poisoned or a validated title is absent.
    pub fn request_control(
        &mut self,
        control: NativeInteractiveControl,
        now_ms: i64,
    ) -> Result<NativeInteractiveControlId, NativeInteractiveError> {
        let control = match control {
            NativeInteractiveControl::Continue { options } => {
                return self.request_continuation(options);
            }
            other => other,
        };
        self.check_control_admission()?;
        let next = self
            .next_control
            .checked_add(1)
            .ok_or(NativeInteractiveError::IdentityExhausted)?;
        let id = NativeInteractiveControlId(self.next_control);
        let source = transition::principal(&self.current);
        let control = match control {
            NativeInteractiveControl::Rename { title } => {
                // Reuse the native title contract before retaining its bounded,
                // normalized value; no record, clock or filesystem is read.
                let mut checked = NativeSessionMetadata::new(
                    &self.options.workspace,
                    now_ms,
                    NativeSessionOrigin::Cli,
                )
                .map_err(|_| NativeInteractiveError::Configuration)?;
                checked
                    .rename(&title, now_ms)
                    .map_err(|_| NativeInteractiveError::Configuration)?;
                NativeInteractiveControl::Rename {
                    title: checked
                        .title()
                        .expect("validated rename has title")
                        .to_owned(),
                }
            }
            other => other,
        };
        let runtime = self.current.clone();
        let future = match control {
            NativeInteractiveControl::Allowlist { request, store } => {
                request
                    .validate_registry(|name| self.host.allowlist_tool_registered(name))
                    .map_err(|_| NativeInteractiveError::Configuration)?;
                if runtime.permissions().is_none() {
                    return Err(NativeInteractiveError::Configuration);
                }
                let future = crate::allowlist::service::execute(
                    runtime,
                    store,
                    self.host.workspace_root().to_path_buf(),
                    self.host
                        .control_workers()
                        .ok_or(NativeInteractiveError::Configuration)?,
                    request,
                );
                Box::pin(async move {
                    future
                        .await
                        .map(NativeInteractiveControlReceipt::Allowlist)
                        .map_err(NativeInteractiveControlError::Allowlist)
                }) as BoxFuture<'static, _>
            }
            NativeInteractiveControl::UndoLast => undo::execute(
                runtime,
                self.host
                    .undo_tracker()
                    .ok_or(NativeInteractiveError::Configuration)?,
                self.host
                    .control_workers()
                    .ok_or(NativeInteractiveError::Configuration)?,
            ),
            other => Box::pin(execute(runtime, other, now_ms)),
        };
        self.next_control = next;
        self.control = Some(OwnedControl { id, source, future });
        self.notify();
        Ok(id)
    }

    fn request_continuation(
        &mut self,
        options: InferenceOptions,
    ) -> Result<NativeInteractiveControlId, NativeInteractiveError> {
        use crate::conversation::{ConversationInput, PendingInput};
        // This guard also releases deeply nested rejected JSON iteratively.
        let mut input = PendingInput(Some(ConversationInput::Continue(options)));
        self.check_control_admission()?;
        let next = self
            .next_control
            .checked_add(1)
            .ok_or(NativeInteractiveError::IdentityExhausted)?;
        let id = NativeInteractiveControlId(self.next_control);
        let Some(ConversationInput::Continue(options)) = input.0.take() else {
            unreachable!("owned continuation");
        };
        let job = self.current.enqueue_continuation(options)?;
        self.next_control = next;
        self.control_outcome = Some(NativeInteractiveControlOutcome {
            id,
            source: transition::principal(&self.current),
            result: Ok(NativeInteractiveControlReceipt::Continued(job)),
        });
        self.notify();
        Ok(id)
    }

    /// Changes accepted runtime selection, not its persistence receipt. Taken
    /// turns keep their earlier snapshot; future jobs see this generation.
    /// # Errors
    /// Rejects unavailable owner lanes or native selection-generation exhaustion.
    /// # Panics
    /// Panics if native runtime state was poisoned.
    pub fn set_model_preferences(
        &mut self,
        preferences: NativeModelPreferences,
    ) -> Result<u64, NativeInteractiveError> {
        self.check_control_admission()?;
        let generation = self.current.set_model_preferences(preferences)?;
        self.notify();
        Ok(generation)
    }

    fn check_control_admission(&self) -> Result<(), NativeInteractiveError> {
        if self.closed || self.shutting_down {
            return Err(NativeInteractiveError::Closed);
        }
        if self.control.is_some()
            || self.control_outcome.is_some()
            || self.outcome.is_some()
            || self.transition.is_some()
            || self.pending.is_some()
        {
            return Err(NativeInteractiveError::Busy);
        }
        Ok(())
    }

    #[must_use]
    pub fn take_control_outcome(&mut self) -> Option<NativeInteractiveControlOutcome> {
        let outcome = self.control_outcome.take();
        if outcome.is_some() {
            self.notify();
        }
        outcome
    }

    pub(super) fn poll_control(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        let Some(control) = &mut self.control else {
            return Poll::Ready(());
        };
        let result = match control.future.as_mut().poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(result) => result,
        };
        let control = self.control.take().expect("polled owned control");
        let outcome = NativeInteractiveControlOutcome {
            id: control.id,
            source: control.source,
            result,
        };
        if outcome.failed()
            && let Some(request) = self.pending.take()
        {
            self.outcome = Some(NativeInteractiveOutcome::Rejected {
                request: request.id,
                error: NativeInteractiveError::ControlFailed,
                settled_turn: None,
                candidate: None,
            });
        }
        self.control_outcome = Some(outcome);
        Poll::Ready(())
    }
}

async fn execute(
    runtime: Arc<NativeConversationRuntime>,
    control: NativeInteractiveControl,
    now_ms: i64,
) -> Result<NativeInteractiveControlReceipt, NativeInteractiveControlError> {
    use NativeInteractiveControlReceipt as Receipt;
    Ok(match control {
        NativeInteractiveControl::Rename { title } => {
            Receipt::Renamed(runtime.rename(&title, now_ms).await?)
        }
        NativeInteractiveControl::Compact => Receipt::Compacted(runtime.compact(now_ms).await?),
        NativeInteractiveControl::SaveModelSession => {
            Receipt::ModelSession(runtime.flush_model_preferences(now_ms).await?)
        }
        NativeInteractiveControl::SaveModelDefaults { store } => {
            Receipt::ModelDefaults(runtime.persist_model_preferences(&store, now_ms).await?)
        }
        NativeInteractiveControl::ConfirmPermissionRule { proposal } => {
            let permissions = runtime
                .permissions()
                .ok_or(NativeInteractiveControlError::Unavailable)?;
            Receipt::PermissionRuleConfirmed(
                permissions
                    .confirm_rule_change(proposal)
                    .await
                    .map_err(NativeInteractiveControlError::Permission)?,
            )
        }
        NativeInteractiveControl::Continue { .. }
        | NativeInteractiveControl::UndoLast
        | NativeInteractiveControl::Allowlist { .. } => {
            unreachable!("specialized control checked before retention")
        }
    })
}

pub(super) mod undo;
