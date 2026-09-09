//! Independent read-only snapshot selection and owned clipboard response lane.

use super::{NativeInteractiveError, NativeInteractiveSession, transition};
use crate::{NativeClipboardError, NativeClipboardReplySelection, NativeClipboardReplyStep};
use machine_god_core::{BackgroundOutputOwner, BoxFuture, CancellationToken};
use std::{
    fmt,
    task::{Context, Poll},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeInteractiveCopyId(u64);
impl NativeInteractiveCopyId {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeInteractiveCopyReceipt {
    Empty,
    Copied,
}

/// Fixed failures never contain copied text or process diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeInteractiveCopyError {
    ResourceLimit,
    Clipboard(NativeClipboardError),
    Cancelled,
}
impl fmt::Display for NativeInteractiveCopyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("native clipboard operation failed")
    }
}
impl std::error::Error for NativeInteractiveCopyError {}

/// Response only: the full host retains actual worker and child-reap ownership.
pub struct NativeInteractiveCopyOutcome {
    pub id: NativeInteractiveCopyId,
    pub source: BackgroundOutputOwner,
    pub result: Result<NativeInteractiveCopyReceipt, NativeInteractiveCopyError>,
}
impl fmt::Debug for NativeInteractiveCopyOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeInteractiveCopyOutcome { .. }")
    }
}
pub(super) struct OwnedCopy {
    id: NativeInteractiveCopyId,
    pub(super) source: BackgroundOutputOwner,
    pub(super) cancel: CancellationToken,
    pub(super) phase: Phase,
}
pub(super) enum Phase {
    Selecting(NativeClipboardReplySelection),
    Copying(BoxFuture<'static, Result<(), NativeClipboardError>>),
}

impl NativeInteractiveSession {
    /// Pins the current principal and canonical snapshot without effects. Copy
    /// admission is independent of model/control queues and unread turn output.
    /// # Errors
    /// Rejects closed/switching owners, occupied copy lanes or identity exhaustion.
    pub fn request_copy(&mut self) -> Result<NativeInteractiveCopyId, NativeInteractiveError> {
        if self.closed || self.shutting_down {
            return Err(NativeInteractiveError::Closed);
        }
        if self.copy.is_some()
            || self.copy_outcome.is_some()
            || self.transition.is_some()
            || self.pending.is_some()
        {
            return Err(NativeInteractiveError::Busy);
        }
        let next = self
            .next_copy
            .checked_add(1)
            .ok_or(NativeInteractiveError::IdentityExhausted)?;
        let id = NativeInteractiveCopyId(self.next_copy);
        self.copy = Some(OwnedCopy {
            id,
            source: transition::principal(&self.current),
            cancel: CancellationToken::new(),
            phase: Phase::Selecting(NativeClipboardReplySelection::new(
                self.current.record_snapshot(),
            )),
        });
        self.next_copy = next;
        self.notify();
        Ok(id)
    }

    /// True while an accepted copy needs polling, even after session closure.
    /// Once false, take its retained outcome before native-free final output.
    /// This is not the full host's worker completion or child-reap receipt.
    #[must_use]
    pub const fn has_pending_copy(&self) -> bool {
        self.copy.is_some()
    }

    #[must_use]
    pub fn take_copy_outcome(&mut self) -> Option<NativeInteractiveCopyOutcome> {
        let outcome = self.copy_outcome.take();
        if outcome.is_some() {
            self.notify();
        }
        outcome
    }

    pub(super) fn cancel_copy(&mut self) {
        let Some(copy) = self.copy.as_ref() else {
            return;
        };
        copy.cancel.cancel();
        if matches!(copy.phase, Phase::Selecting(_)) {
            let copy = self.copy.take().expect("observed copy remains owned");
            self.finish_copy(copy, Err(NativeInteractiveCopyError::Cancelled));
        }
    }

    pub(super) fn poll_copy(&mut self, cx: &mut Context<'_>) {
        let Some(mut copy) = self.copy.take() else {
            return;
        };
        let result = match &mut copy.phase {
            Phase::Selecting(selection) => match selection.next_step() {
                Ok(NativeClipboardReplyStep::Progress) => None,
                Ok(NativeClipboardReplyStep::Empty) => {
                    Some(Ok(NativeInteractiveCopyReceipt::Empty))
                }
                Ok(NativeClipboardReplyStep::Selected(text)) => match &self.clipboard {
                    Ok(clipboard) => {
                        copy.phase = Phase::Copying(clipboard.copy(text, copy.cancel.clone()));
                        None
                    }
                    Err(error) => Some(Err(NativeInteractiveCopyError::Clipboard(*error))),
                },
                Err(_) => Some(Err(NativeInteractiveCopyError::ResourceLimit)),
            },
            Phase::Copying(future) => match future.as_mut().poll(cx) {
                Poll::Pending => {
                    self.copy = Some(copy);
                    return;
                }
                Poll::Ready(result) => Some(
                    result
                        .map(|()| NativeInteractiveCopyReceipt::Copied)
                        .map_err(|error| {
                            if error == NativeClipboardError::Cancelled {
                                NativeInteractiveCopyError::Cancelled
                            } else {
                                NativeInteractiveCopyError::Clipboard(error)
                            }
                        }),
                ),
            },
        };
        if let Some(result) = result {
            self.finish_copy(copy, result);
        } else {
            self.copy = Some(copy);
            cx.waker().wake_by_ref();
        }
    }

    fn finish_copy(
        &mut self,
        copy: OwnedCopy,
        result: Result<NativeInteractiveCopyReceipt, NativeInteractiveCopyError>,
    ) {
        debug_assert!(self.copy_outcome.is_none());
        self.copy_outcome = Some(NativeInteractiveCopyOutcome {
            id: copy.id,
            source: copy.source,
            result,
        });
    }
}
