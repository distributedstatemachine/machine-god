use super::{
    DeliveryRecord, NOTICE_CONTEXT_KEY, NOTICE_OUTBOX_KEY, NoticeContextError,
    PreparedNoticeContext, Slot, checkpoint::bounded_value,
};
use machine_god_core::{
    BoxFuture, EngineError, InferenceOptions, Prompt, Session, SessionTurnPreparation, Turn,
};
use std::{fmt, sync::Arc};

#[derive(Debug)]
pub(crate) enum NoticePublicationError {
    Context(NoticeContextError),
    Core(EngineError),
}
impl fmt::Display for NoticePublicationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("managed notice prompt publication failed")
    }
}
impl std::error::Error for NoticePublicationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Context(error) => Some(error),
            Self::Core(error) => Some(error),
        }
    }
}
impl PreparedNoticeContext {
    pub(crate) fn publish_prompt(
        self,
        session: &Session,
        prompt: Prompt,
        preparation: SessionTurnPreparation,
    ) -> BoxFuture<'static, Result<Turn, NoticePublicationError>> {
        let validation = self.validate_preparation(session, &preparation).and({
            if self.recovering {
                Err(NoticeContextError::Uncertain)
            } else {
                Ok(())
            }
        });
        // Core takes ownership before rejection, including its iterative JSON drop guards.
        let future = session.prompt_prepared(prompt, preparation);
        self.publish(validation, future)
    }
    pub(crate) fn publish_continuation(
        self,
        session: &Session,
        options: InferenceOptions,
        preparation: SessionTurnPreparation,
    ) -> BoxFuture<'static, Result<Turn, NoticePublicationError>> {
        let validation = self.validate_preparation(session, &preparation).and({
            if self.recovering {
                Ok(())
            } else {
                Err(NoticeContextError::InvalidCheckpoint)
            }
        });
        let future = session.continue_turn_prepared(options, preparation);
        self.publish(validation, future)
    }
    fn validate_preparation(
        &self,
        session: &Session,
        preparation: &SessionTurnPreparation,
    ) -> Result<(), NoticeContextError> {
        let owner = self.owner.upgrade().ok_or(NoticeContextError::Retired)?;
        if !owner.session.same_session(&session.witness()) {
            return Err(NoticeContextError::ForeignSession);
        }
        let value = preparation
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get(NOTICE_CONTEXT_KEY))
            .ok_or(NoticeContextError::InvalidCheckpoint)?;
        bounded_value(value)?;
        let outbox = preparation
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get(NOTICE_OUTBOX_KEY))
            .ok_or(NoticeContextError::InvalidCheckpoint)?;
        bounded_value(outbox)?;
        if preparation.expected_revision != self.checkpoint.expected_revision
            || value != &self.checkpoint_value()?
            || outbox != &self.outbox_value()?
            || preparation.user_context.as_ref().is_none_or(|context| {
                context.user_message_index != self.checkpoint.first_user_message
                    || context.text != self.text
            })
        {
            return Err(NoticeContextError::InvalidCheckpoint);
        }
        Ok(())
    }
    fn publish(
        mut self,
        validation: Result<(), NoticeContextError>,
        future: BoxFuture<'static, Result<Turn, EngineError>>,
    ) -> BoxFuture<'static, Result<Turn, NoticePublicationError>> {
        Box::pin(async move {
            validation.map_err(NoticePublicationError::Context)?;
            self.begin().map_err(NoticePublicationError::Context)?;
            let turn = future.await.map_err(NoticePublicationError::Core)?;
            // Successful core publication is the only acknowledgement evidence.
            let live = self.validate_live();
            let parent_live = self.confirm();
            if let Err(error) = live.and({
                if parent_live {
                    Ok(())
                } else {
                    Err(NoticeContextError::Retired)
                }
            }) {
                let _ = turn.handle().cancel();
                return Err(NoticePublicationError::Context(error));
            }
            Ok(turn)
        })
    }
    fn validate_live(&self) -> Result<(), NoticeContextError> {
        let owner = self.owner.upgrade().ok_or(NoticeContextError::Retired)?;
        let notices = owner.notices.upgrade().ok_or(NoticeContextError::Retired)?;
        notices
            .validate_batch(&self.payload.batch)
            .map_err(|_| NoticeContextError::Stale)?;
        let slot = owner
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if matches!(&*slot, Slot::Publishing(payload) if Arc::ptr_eq(payload, &self.payload)) {
            Ok(())
        } else {
            Err(NoticeContextError::Retired)
        }
    }
    fn begin(&mut self) -> Result<(), NoticeContextError> {
        let owner = self.owner.upgrade().ok_or(NoticeContextError::Retired)?;
        let notices = owner.notices.upgrade().ok_or(NoticeContextError::Retired)?;
        notices
            .validate_batch(&self.payload.batch)
            .map_err(|_| NoticeContextError::Stale)?;
        let mut slot = owner
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !matches!(&*slot, Slot::Prepared(payload) if Arc::ptr_eq(payload, &self.payload)) {
            return Err(NoticeContextError::Retired);
        }
        *slot = Slot::Publishing(Arc::clone(&self.payload));
        self.started = true;
        Ok(())
    }
    fn confirm(&mut self) -> bool {
        let mut parent_live = false;
        if let Some(owner) = self.owner.upgrade() {
            if let Some(notices) = owner.notices.upgrade() {
                // A concurrent source close can remove a selected original. Ack each
                // exact original independently so it cannot strand confirmed siblings.
                for token in &self.payload.tokens {
                    let _ = notices.acknowledge(&self.payload.batch, std::slice::from_ref(token));
                }
            }
            let mut slot = owner
                .slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if matches!(&*slot, Slot::Publishing(payload) if Arc::ptr_eq(payload, &self.payload)) {
                *slot = Slot::Delivered(DeliveryRecord::confirmed(self.payload.outbox.clone()));
                parent_live = true;
            }
        }
        self.finished = true;
        parent_live
    }
}
impl Drop for PreparedNoticeContext {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let mut slot = owner
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if matches!(&*slot, Slot::Prepared(payload) | Slot::Publishing(payload) if Arc::ptr_eq(payload, &self.payload))
        {
            *slot = if self.started || self.recovering {
                Slot::Uncertain(Arc::clone(&self.payload))
            } else {
                Slot::Idle
            };
        }
    }
}
