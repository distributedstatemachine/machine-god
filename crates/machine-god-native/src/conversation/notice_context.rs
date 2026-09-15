//! Next-turn notice context under the conversation's actual admission lease.

use super::{
    Checkpoint, ConversationInput, NativeConversation, NativeConversationError, map_engine_error,
};
use crate::managed::prompt_context::{
    self, NOTICE_CONTEXT_KEY, NOTICE_OUTBOX_KEY, NoticeCheckpoint, NoticeDelivery,
    NoticePublicationError, ParentNoticeContext, PreparedNoticeContext,
};
use machine_god_core::{
    BoxFuture, Session, SessionRecord, SessionRevision, SessionTurnPreparation, SessionUserContext,
    Turn,
};
use std::sync::Arc;

impl NativeConversation {
    /// Explicit metadata-only durability repair; never starts a parent turn.
    pub(crate) fn recover_notice_delivery(
        &self,
    ) -> BoxFuture<'_, Result<Option<NoticeDelivery>, NativeConversationError>> {
        Box::pin(async move {
            let _lifecycle = self.acquire_lifecycle()?;
            let _admission = self.acquire_workspace_control()?;
            let owner = self
                .notices
                .as_ref()
                .and_then(std::sync::Weak::upgrade)
                .ok_or(NativeConversationError::ManagedAdmission)?;
            owner
                .recover_delivery(&self.session)
                .await
                .map_err(publication_error)
        })
    }

    /// Clears only the original batch after all exact source ACKs are confirmed.
    pub(crate) fn clear_notice_delivery<'a>(
        &'a self,
        delivery: &'a NoticeDelivery,
    ) -> BoxFuture<'a, Result<SessionRevision, NativeConversationError>> {
        Box::pin(async move {
            let _lifecycle = self.acquire_lifecycle()?;
            let _admission = self.acquire_workspace_control()?;
            let owner = self
                .notices
                .as_ref()
                .and_then(std::sync::Weak::upgrade)
                .ok_or(NativeConversationError::ManagedAdmission)?;
            owner
                .clear_delivery(&self.session, delivery)
                .await
                .map_err(publication_error)
        })
    }
    pub(crate) fn with_notice_context(
        mut self,
        owner: &Arc<ParentNoticeContext>,
    ) -> Result<Self, NativeConversationError> {
        if self.is_busy() || self.notices.is_some() {
            return Err(NativeConversationError::Busy);
        }
        owner
            .needs_reconciliation(&self.session)
            .map_err(rejected)?;
        self.notices = Some(Arc::downgrade(owner));
        Ok(self)
    }

    pub(super) fn prepare_notice_context(
        &self,
        record: &mut SessionRecord,
        previous: Option<Checkpoint>,
        checkpoint: Checkpoint,
        is_prompt: bool,
        context: &mut Option<SessionUserContext>,
    ) -> Result<Option<PreparedNoticeContext>, NativeConversationError> {
        let parent = self
            .notices
            .as_ref()
            .map(|owner| {
                owner
                    .upgrade()
                    .ok_or(NativeConversationError::ManagedAdmission)
            })
            .transpose()?;
        let next = NoticeCheckpoint {
            session_id: record.id.clone(),
            incarnation_id: record.incarnation_id.clone(),
            expected_revision: record.revision,
            turn_sequence: checkpoint.turn_sequence,
            first_user_message: checkpoint.first_user_message,
        };
        let saved = prompt_context::saved_context(
            record,
            previous.map(|value| (value.turn_sequence, value.first_user_message)),
        )
        .map_err(rejected)?;
        let base = context.as_ref().map(|context| context.text.as_str());
        let prepared = match &parent {
            Some(parent) if is_prompt => parent
                .prepare(&self.session, record, next.clone(), base, None)
                .map_err(rejected)?,
            Some(parent)
                if parent
                    .needs_reconciliation(&self.session)
                    .map_err(rejected)? =>
            {
                Some(
                    parent
                        .prepare_continuation(
                            &self.session,
                            record,
                            saved
                                .as_ref()
                                .ok_or(NativeConversationError::ManagedAdmission)?,
                            next.clone(),
                            base,
                            None,
                        )
                        .map_err(rejected)?,
                )
            }
            _ => None,
        };
        record.metadata.remove(NOTICE_CONTEXT_KEY);
        if let Some(prepared) = &prepared {
            *context = Some(prepared.user_context());
            record.metadata.insert(
                NOTICE_CONTEXT_KEY.to_owned(),
                prepared.checkpoint_value().map_err(rejected)?,
            );
            record.metadata.insert(
                NOTICE_OUTBOX_KEY.to_owned(),
                prepared.outbox_value().map_err(rejected)?,
            );
        } else if !is_prompt && let Some(saved) = saved {
            *context = prompt_context::compose_user_context(
                base,
                None,
                Some(saved.text()),
                checkpoint.first_user_message,
            )
            .map_err(rejected)?;
            record.metadata.insert(
                NOTICE_CONTEXT_KEY.to_owned(),
                saved.at_checkpoint(next).to_value().map_err(rejected)?,
            );
        }
        Ok(prepared)
    }
}

pub(super) async fn publish(
    prepared: Option<PreparedNoticeContext>,
    session: &Session,
    input: ConversationInput,
    preparation: SessionTurnPreparation,
) -> Result<Turn, NativeConversationError> {
    // Confirmation/ACK happens inside these wrappers immediately after the core
    // checkpoint commits, before any later native route registration can fail.
    match (prepared, input) {
        (Some(notice), ConversationInput::Prompt(prompt)) => notice
            .publish_prompt(session, prompt, preparation)
            .await
            .map_err(publication_error),
        (Some(notice), ConversationInput::Continue(options)) => notice
            .publish_continuation(session, options, preparation)
            .await
            .map_err(publication_error),
        (None, ConversationInput::Prompt(prompt)) => session
            .prompt_prepared(prompt, preparation)
            .await
            .map_err(map_engine_error),
        (None, ConversationInput::Continue(options)) => session
            .continue_turn_prepared(options, preparation)
            .await
            .map_err(map_engine_error),
    }
}

pub(super) fn validate_saved_context(
    record: &SessionRecord,
    checkpoint: Option<Checkpoint>,
    skill: Option<&str>,
    resource: Option<&str>,
) -> Result<(), NativeConversationError> {
    prompt_context::saved_outbox(record).map_err(rejected)?;
    let saved = prompt_context::saved_context(
        record,
        checkpoint.map(|value| (value.turn_sequence, value.first_user_message)),
    )
    .map_err(rejected)?;
    prompt_context::compose_user_context(
        skill,
        resource,
        saved.as_ref().map(prompt_context::SavedNoticeContext::text),
        checkpoint.map_or(0, |value| value.first_user_message),
    )
    .map_err(rejected)?;
    Ok(())
}

fn rejected(_: prompt_context::NoticeContextError) -> NativeConversationError {
    NativeConversationError::ManagedAdmission
}
fn publication_error(error: NoticePublicationError) -> NativeConversationError {
    match error {
        NoticePublicationError::Context(error) => rejected(error),
        NoticePublicationError::Core(error) => map_engine_error(error),
    }
}
