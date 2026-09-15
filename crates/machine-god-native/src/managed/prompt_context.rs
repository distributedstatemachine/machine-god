//! Explicit next-turn notice preparation, never an idle-turn trigger.
mod checkpoint;
#[cfg(test)]
mod conversation_tests;
mod publication;
#[cfg(test)]
mod tests;

use super::notices::{ManagedNotices, NoticeAckToken, NoticeBatch, NoticePrincipal};
pub(crate) use checkpoint::{
    NOTICE_CONTEXT_KEY, NoticeCheckpoint, SavedNoticeContext, compose_user_context, saved_context,
};
use machine_god_core::{Session, SessionRecord, SessionUserContext, SessionWitness};
pub(crate) use publication::NoticePublicationError;
use std::{
    fmt,
    sync::{Arc, Mutex, Weak},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NoticeContextError {
    Busy,
    Uncertain,
    Retired,
    InvalidCheckpoint,
    ResourceLimit,
    Stale,
    ForeignSession,
}
impl fmt::Display for NoticeContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("managed notice prompt context rejected")
    }
}
impl std::error::Error for NoticeContextError {}

/// One bounded slot per manager-admitted parent. IDs are correlation, not authority.
pub(crate) struct ParentNoticeContext {
    inner: Arc<Inner>,
}
struct Inner {
    session: SessionWitness,
    parent: NoticePrincipal,
    notices: Weak<ManagedNotices>,
    slot: Mutex<Slot>,
}
enum Slot {
    Idle,
    Prepared(Arc<Payload>),
    Publishing(Arc<Payload>),
    Uncertain(Arc<Payload>),
    Retired,
}
struct Payload {
    batch: NoticeBatch,
    tokens: Vec<NoticeAckToken>,
    saved: SavedNoticeContext,
}

pub(crate) struct PreparedNoticeContext {
    owner: Weak<Inner>,
    payload: Arc<Payload>,
    checkpoint: NoticeCheckpoint,
    text: String,
    recovering: bool,
    started: bool,
    finished: bool,
}
impl fmt::Debug for PreparedNoticeContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PreparedNoticeContext(..)")
    }
}
impl ParentNoticeContext {
    pub(crate) fn new(
        session: &Session,
        parent: NoticePrincipal,
        notices: &Arc<ManagedNotices>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                session: session.witness(),
                parent,
                notices: Arc::downgrade(notices),
                slot: Mutex::new(Slot::Idle),
            }),
        }
    }
    /// Root holds its actual parent lifecycle admission throughout preparation/publication.
    pub(crate) fn prepare(
        &self,
        session: &Session,
        record: &SessionRecord,
        checkpoint: NoticeCheckpoint,
        skill: Option<&str>,
        resource: Option<&str>,
    ) -> Result<Option<PreparedNoticeContext>, NoticeContextError> {
        self.validate_session(session)?;
        checkpoint.validate_record(record)?;
        checkpoint.validate_record(&session.record())?;
        let base = compose_user_context(skill, resource, None, checkpoint.first_user_message)?;
        let base_text = base.as_ref().map(|context| context.text.as_str());
        let notices = self
            .inner
            .notices
            .upgrade()
            .ok_or(NoticeContextError::Retired)?;
        let mut slot = self
            .inner
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &*slot {
            Slot::Idle => {}
            Slot::Uncertain(_) => return Err(NoticeContextError::Uncertain),
            Slot::Retired => return Err(NoticeContextError::Retired),
            _ => return Err(NoticeContextError::Busy),
        }
        let batch = notices
            .snapshot(
                &self.inner.parent,
                64,
                machine_god_core::MAX_SESSION_USER_CONTEXT_BYTES,
            )
            .map_err(|_| NoticeContextError::Stale)?;
        let mut originals = Vec::new();
        let mut tokens = Vec::new();
        let mut selected = None;
        for entry in batch.entries() {
            originals.push(entry.notice().clone());
            let saved =
                match SavedNoticeContext::new(&self.inner.parent, &checkpoint, originals.clone()) {
                    Ok(saved) => saved,
                    Err(NoticeContextError::ResourceLimit) => break,
                    Err(error) => return Err(error),
                };
            let Ok(Some(context)) = compose_user_context(
                base_text,
                None,
                Some(saved.text()),
                checkpoint.first_user_message,
            ) else {
                break;
            };
            tokens.push(entry.token());
            selected = Some((saved, context.text));
        }
        let Some((saved, text)) = selected else {
            return Ok(None);
        };
        let payload = Arc::new(Payload {
            batch,
            tokens,
            saved,
        });
        *slot = Slot::Prepared(Arc::clone(&payload));
        Ok(Some(PreparedNoticeContext {
            owner: Arc::downgrade(&self.inner),
            payload,
            checkpoint,
            text,
            recovering: false,
            started: false,
            finished: false,
        }))
    }
    /// Readback supplies inert originals only. This does not acknowledge or clear a fence.
    pub(crate) fn prepare_continuation(
        &self,
        session: &Session,
        record: &SessionRecord,
        saved: &SavedNoticeContext,
        checkpoint: NoticeCheckpoint,
        skill: Option<&str>,
        resource: Option<&str>,
    ) -> Result<PreparedNoticeContext, NoticeContextError> {
        self.validate_session(session)?;
        checkpoint.validate_record(record)?;
        let actual = session.record();
        checkpoint.validate_record(&actual)?;
        if saved_context(
            &actual,
            Some((
                saved.checkpoint.turn_sequence,
                saved.checkpoint.first_user_message,
            )),
        )?
        .as_ref()
            != Some(saved)
        {
            return Err(NoticeContextError::InvalidCheckpoint);
        }
        let mut slot = self
            .inner
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Slot::Uncertain(payload) = &*slot else {
            return Err(NoticeContextError::Uncertain);
        };
        if !payload.saved.same_originals(saved)
            || saved.parent != self.inner.parent
            || saved.checkpoint.first_user_message != checkpoint.first_user_message
        {
            return Err(NoticeContextError::InvalidCheckpoint);
        }
        let text = compose_user_context(
            skill,
            resource,
            Some(saved.text()),
            checkpoint.first_user_message,
        )?
        .ok_or(NoticeContextError::InvalidCheckpoint)?
        .text;
        let payload = Arc::clone(payload);
        *slot = Slot::Prepared(Arc::clone(&payload));
        Ok(PreparedNoticeContext {
            owner: Arc::downgrade(&self.inner),
            payload,
            checkpoint,
            text,
            recovering: true,
            started: false,
            finished: false,
        })
    }
    pub(crate) fn retire(&self) {
        let old = std::mem::replace(
            &mut *self
                .inner
                .slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            Slot::Retired,
        );
        drop(old);
    }
    pub(crate) fn needs_reconciliation(
        &self,
        session: &Session,
    ) -> Result<bool, NoticeContextError> {
        self.validate_session(session)?;
        match &*self
            .inner
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            Slot::Idle => Ok(false),
            Slot::Uncertain(_) => Ok(true),
            Slot::Retired => Err(NoticeContextError::Retired),
            _ => Err(NoticeContextError::Busy),
        }
    }

    pub(crate) fn validate_session(&self, session: &Session) -> Result<(), NoticeContextError> {
        if self.inner.session.same_session(&session.witness()) && self.inner.session.is_live() {
            Ok(())
        } else {
            Err(NoticeContextError::ForeignSession)
        }
    }
}
impl PreparedNoticeContext {
    pub(crate) fn user_context(&self) -> SessionUserContext {
        SessionUserContext {
            user_message_index: self.checkpoint.first_user_message,
            text: self.text.clone(),
        }
    }
    pub(crate) fn checkpoint_value(&self) -> Result<serde_json::Value, NoticeContextError> {
        self.payload
            .saved
            .at_checkpoint(self.checkpoint.clone())
            .to_value()
    }
}
