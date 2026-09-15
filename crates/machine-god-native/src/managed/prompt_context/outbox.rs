//! One original delivery batch, retained independently of removable prompt context.
use super::super::notices::{ManagedNotice, NoticeIdentity, NoticePrincipal};
use super::{
    Inner, NoticeCheckpoint, NoticeContextError, NoticePublicationError, ParentNoticeContext,
    SavedNoticeContext, Slot, checkpoint::bounded_value,
};
use machine_god_core::{Session, SessionRecord, SessionRevision};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fmt,
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
};

pub(crate) const NOTICE_OUTBOX_KEY: &str = "machine_god.managed_notice_delivery_outbox";
const MAX_OUTBOX_BYTES: usize = 64 * 1024;
const MAX_NOTICE_METADATA_BYTES: usize = 192 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SavedNoticeOutbox {
    schema_version: u8,
    parent: NoticePrincipal,
    checkpoint: NoticeCheckpoint,
    originals: Vec<ManagedNotice>,
}
impl SavedNoticeOutbox {
    pub(super) fn new(saved: &SavedNoticeContext) -> Result<Self, NoticeContextError> {
        let outbox = Self {
            schema_version: 1,
            parent: saved.parent.clone(),
            checkpoint: saved.checkpoint.clone(),
            originals: saved.originals().to_vec(),
        };
        let outbox_bytes = serde_json::to_vec(&outbox)
            .map_err(|_| NoticeContextError::InvalidCheckpoint)?
            .len();
        let context_bytes = serde_json::to_vec(&saved.to_value()?)
            .map_err(|_| NoticeContextError::InvalidCheckpoint)?
            .len();
        if outbox_bytes > MAX_OUTBOX_BYTES
            || outbox_bytes
                .checked_add(context_bytes)
                .is_none_or(|size| size > MAX_NOTICE_METADATA_BYTES)
        {
            return Err(NoticeContextError::ResourceLimit);
        }
        Ok(outbox)
    }
    pub(super) fn to_value(&self) -> Result<Value, NoticeContextError> {
        let bytes = serde_json::to_vec(self).map_err(|_| NoticeContextError::InvalidCheckpoint)?;
        if bytes.len() > MAX_OUTBOX_BYTES {
            return Err(NoticeContextError::ResourceLimit);
        }
        serde_json::from_slice(&bytes).map_err(|_| NoticeContextError::InvalidCheckpoint)
    }
}
/// Inert decoding only. This never mints a delivery receipt or clears uncertainty.
pub(crate) fn saved_outbox(
    record: &SessionRecord,
) -> Result<Option<SavedNoticeOutbox>, NoticeContextError> {
    let Some(value) = record.metadata.get(NOTICE_OUTBOX_KEY) else {
        return Ok(None);
    };
    bounded_value(value)?;
    if serde_json::to_vec(value)
        .map_err(|_| NoticeContextError::InvalidCheckpoint)?
        .len()
        > MAX_OUTBOX_BYTES
    {
        return Err(NoticeContextError::ResourceLimit);
    }
    let outbox: SavedNoticeOutbox =
        serde_json::from_value(value.clone()).map_err(|_| NoticeContextError::InvalidCheckpoint)?;
    if outbox.schema_version != 1
        || outbox.originals.is_empty()
        || outbox.originals.len() > 64
        || outbox.checkpoint.session_id != record.id
        || outbox.checkpoint.incarnation_id != record.incarnation_id
        || outbox.checkpoint.expected_revision >= record.revision
        || outbox.checkpoint.turn_sequence == 0
        || outbox.checkpoint.turn_sequence >= record.next_turn_sequence
        || outbox.checkpoint.first_user_message >= record.messages.len()
        || outbox
            .originals
            .iter()
            .any(|notice| notice.target.parent != outbox.parent)
    {
        return Err(NoticeContextError::InvalidCheckpoint);
    }
    for (index, notice) in outbox.originals.iter().enumerate() {
        if outbox.originals[..index]
            .iter()
            .any(|other| other.identity() == notice.identity())
        {
            return Err(NoticeContextError::InvalidCheckpoint);
        }
    }
    Ok(Some(outbox))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NoticeDeliveryProvenance {
    ConfirmedPublication,
    RecoveredOriginal,
}
pub(super) struct DeliveryRecord {
    outbox: SavedNoticeOutbox,
    provenance: NoticeDeliveryProvenance,
    acknowledged: AtomicU64,
}
impl DeliveryRecord {
    pub(super) fn confirmed(outbox: SavedNoticeOutbox) -> Arc<Self> {
        Arc::new(Self {
            outbox,
            provenance: NoticeDeliveryProvenance::ConfirmedPublication,
            acknowledged: AtomicU64::new(0),
        })
    }
    fn complete_mask(&self) -> u64 {
        if self.outbox.originals.len() == 64 {
            u64::MAX
        } else {
            (1u64 << self.outbox.originals.len()) - 1
        }
    }
}
/// Weak exact receipt, not session/runtime ownership or a new notice occurrence.
pub(crate) struct NoticeDelivery {
    owner: Weak<Inner>,
    record: Arc<DeliveryRecord>,
}
impl NoticeDelivery {
    pub(crate) fn originals(&self) -> &[ManagedNotice] {
        &self.record.outbox.originals
    }
    pub(crate) fn checkpoint(&self) -> &NoticeCheckpoint {
        &self.record.outbox.checkpoint
    }
    pub(crate) fn provenance(&self) -> NoticeDeliveryProvenance {
        self.record.provenance
    }
}
impl fmt::Debug for NoticeDelivery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NoticeDelivery(..)")
    }
}
fn current_delivery(slot: &Slot) -> Option<&Arc<DeliveryRecord>> {
    match slot {
        Slot::Delivered(record) | Slot::ClearUncertain(record) => Some(record),
        _ => None,
    }
}
impl ParentNoticeContext {
    /// Includes in-flight and uncertain custody, not only observable receipts.
    pub(crate) fn has_pending_delivery(&self) -> bool {
        !matches!(
            *self
                .inner
                .slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            Slot::Idle | Slot::Retired
        )
    }

    /// Observes only a previously confirmed publication/repair, never a raw saved record.
    pub(crate) fn delivery(&self) -> Option<NoticeDelivery> {
        if !self.inner.session.is_live() {
            return None;
        }
        let slot = self
            .inner
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        current_delivery(&slot).map(|record| NoticeDelivery {
            owner: Arc::downgrade(&self.inner),
            record: Arc::clone(record),
        })
    }
    /// Manager-only after each exact source-journal ACK (or its confirmed durability repair).
    pub(in crate::managed) fn confirm_source_acknowledgements(
        &self,
        delivery: &NoticeDelivery,
        identities: &[NoticeIdentity],
    ) -> Result<(), NoticeContextError> {
        let slot = self
            .inner
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.validate_delivery(delivery, &slot)?;
        if identities.len() > delivery.originals().len() {
            return Err(NoticeContextError::InvalidCheckpoint);
        }
        let mut mask = 0u64;
        for identity in identities {
            let index = delivery
                .originals()
                .iter()
                .position(|notice| notice.identity() == *identity)
                .ok_or(NoticeContextError::InvalidCheckpoint)?;
            let bit = 1u64 << index;
            if mask & bit != 0 {
                return Err(NoticeContextError::InvalidCheckpoint);
            }
            mask |= bit;
        }
        delivery
            .record
            .acknowledged
            .fetch_or(mask, Ordering::AcqRel);
        Ok(())
    }
    fn validate_delivery(
        &self,
        delivery: &NoticeDelivery,
        slot: &Slot,
    ) -> Result<(), NoticeContextError> {
        if !self.inner.session.is_live()
            || !std::ptr::eq(Arc::as_ptr(&self.inner), delivery.owner.as_ptr())
            || current_delivery(slot).is_none_or(|record| !Arc::ptr_eq(record, &delivery.record))
        {
            return Err(NoticeContextError::Stale);
        }
        Ok(())
    }
    /// Called only inside actual `NativeConversation` lifecycle/admission custody.
    pub(crate) async fn recover_delivery(
        &self,
        session: &Session,
    ) -> Result<Option<NoticeDelivery>, NoticePublicationError> {
        self.validate_session(session)
            .map_err(NoticePublicationError::Context)?;
        let record = session.record();
        let saved = saved_outbox(&record).map_err(NoticePublicationError::Context)?;
        if saved
            .as_ref()
            .is_some_and(|saved| saved.parent != self.inner.parent)
        {
            return Err(NoticePublicationError::Context(
                NoticeContextError::InvalidCheckpoint,
            ));
        }
        let payload = {
            let mut slot = self
                .inner
                .slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match &*slot {
                Slot::Delivered(original) => {
                    if saved.as_ref() != Some(&original.outbox) {
                        return Err(NoticePublicationError::Context(
                            NoticeContextError::InvalidCheckpoint,
                        ));
                    }
                    drop(slot);
                    return Ok(self.delivery());
                }
                Slot::ClearUncertain(original) => {
                    if saved
                        .as_ref()
                        .is_some_and(|saved| saved != &original.outbox)
                    {
                        return Err(NoticePublicationError::Context(
                            NoticeContextError::InvalidCheckpoint,
                        ));
                    }
                    drop(slot);
                    return Ok(self.delivery());
                }
                Slot::Uncertain(_) => {
                    return Err(NoticePublicationError::Context(
                        NoticeContextError::Uncertain,
                    ));
                }
                Slot::Retired => {
                    return Err(NoticePublicationError::Context(NoticeContextError::Retired));
                }
                Slot::RecoveryUncertain(original) => {
                    if saved.as_ref() != Some(&original.outbox) {
                        return Err(NoticePublicationError::Context(
                            NoticeContextError::InvalidCheckpoint,
                        ));
                    }
                    let original = Arc::clone(original);
                    *slot = Slot::Recovering(Arc::clone(&original));
                    original
                }
                Slot::Idle => {
                    let Some(outbox) = saved else {
                        return Ok(None);
                    };
                    let original = Arc::new(DeliveryRecord {
                        outbox,
                        provenance: NoticeDeliveryProvenance::RecoveredOriginal,
                        acknowledged: AtomicU64::new(0),
                    });
                    *slot = Slot::Recovering(Arc::clone(&original));
                    original
                }
                _ => return Err(NoticePublicationError::Context(NoticeContextError::Busy)),
            }
        };
        let mut operation = OutboxOperation {
            owner: Arc::downgrade(&self.inner),
            record: payload,
            kind: OperationKind::Recover,
            finished: false,
        };
        // Even unchanged metadata is saved by core, confirming file/directory durability.
        session
            .update_metadata(record.revision, record.metadata)
            .await
            .map_err(NoticePublicationError::Core)?;
        operation
            .finish()
            .map_err(NoticePublicationError::Context)?;
        Ok(self.delivery())
    }
    /// Called only inside actual `NativeConversation` lifecycle/admission custody.
    pub(crate) async fn clear_delivery(
        &self,
        session: &Session,
        delivery: &NoticeDelivery,
    ) -> Result<SessionRevision, NoticePublicationError> {
        self.validate_session(session)
            .map_err(NoticePublicationError::Context)?;
        let mut record = session.record();
        let saved = saved_outbox(&record).map_err(NoticePublicationError::Context)?;
        {
            let mut slot = self
                .inner
                .slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.validate_delivery(delivery, &slot)
                .map_err(NoticePublicationError::Context)?;
            if delivery.record.acknowledged.load(Ordering::Acquire)
                != delivery.record.complete_mask()
            {
                return Err(NoticePublicationError::Context(NoticeContextError::Busy));
            }
            let retry_absent = matches!(&*slot, Slot::ClearUncertain(_)) && saved.is_none();
            if !retry_absent && saved.as_ref() != Some(&delivery.record.outbox) {
                return Err(NoticePublicationError::Context(
                    NoticeContextError::InvalidCheckpoint,
                ));
            }
            *slot = Slot::Clearing(Arc::clone(&delivery.record));
        }
        let mut operation = OutboxOperation {
            owner: Arc::downgrade(&self.inner),
            record: Arc::clone(&delivery.record),
            kind: OperationKind::Clear,
            finished: false,
        };
        record.metadata.remove(NOTICE_OUTBOX_KEY);
        let revision = session
            .update_metadata(record.revision, record.metadata)
            .await
            .map_err(NoticePublicationError::Core)?;
        operation
            .finish()
            .map_err(NoticePublicationError::Context)?;
        Ok(revision)
    }
}
#[derive(Clone, Copy)]
enum OperationKind {
    Recover,
    Clear,
}
struct OutboxOperation {
    owner: Weak<Inner>,
    record: Arc<DeliveryRecord>,
    kind: OperationKind,
    finished: bool,
}
impl OutboxOperation {
    fn matches(&self, slot: &Slot) -> bool {
        match (&self.kind, slot) {
            (OperationKind::Recover, Slot::Recovering(record))
            | (OperationKind::Clear, Slot::Clearing(record)) => Arc::ptr_eq(record, &self.record),
            _ => false,
        }
    }
    fn finish(&mut self) -> Result<(), NoticeContextError> {
        let owner = self.owner.upgrade().ok_or(NoticeContextError::Retired)?;
        let mut slot = owner
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.matches(&slot) {
            return Err(NoticeContextError::Retired);
        }
        *slot = match self.kind {
            OperationKind::Recover => Slot::Delivered(Arc::clone(&self.record)),
            OperationKind::Clear => Slot::Idle,
        };
        self.finished = true;
        Ok(())
    }
}
impl Drop for OutboxOperation {
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
        if self.matches(&slot) {
            *slot = match self.kind {
                OperationKind::Recover => Slot::RecoveryUncertain(Arc::clone(&self.record)),
                OperationKind::Clear => Slot::ClearUncertain(Arc::clone(&self.record)),
            };
        }
    }
}
