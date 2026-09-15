use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn publish(f: &Fixture) {
    let prepared = f.prepare();
    let prep = preparation(&prepared, &f.record());
    drop(block_on(prepared.publish_prompt(&f.session, "hello".into(), prep)).unwrap());
}
fn acknowledge(parent: &ParentNoticeContext, delivery: &NoticeDelivery) {
    parent
        .confirm_source_acknowledgements(
            delivery,
            &delivery
                .originals()
                .iter()
                .map(ManagedNotice::identity)
                .collect::<Vec<_>>(),
        )
        .unwrap();
}
fn ordinary(f: &Fixture) {
    let mut record = f.record();
    record.metadata.remove(NOTICE_CONTEXT_KEY);
    assert!(
        f.parent
            .prepare(&f.session, &record, checkpoint(&record), None, None)
            .unwrap()
            .is_none()
    );
    let preparation = SessionTurnPreparation {
        expected_revision: record.revision,
        metadata: Some(record.metadata),
        context: None,
        user_context: None,
    };
    drop(block_on(f.session.prompt_prepared("ordinary input", preparation)).unwrap());
}
#[test]
fn original_outbox_survives_ordinary_prompt_and_requires_exact_source_ack_to_clear() {
    let f = Fixture::new();
    publish(&f);
    let delivery = f.parent.delivery().unwrap();
    assert_eq!(
        delivery.provenance(),
        NoticeDeliveryProvenance::ConfirmedPublication
    );
    let original = saved_outbox(&f.record()).unwrap();
    ordinary(&f);
    assert!(!f.record().metadata.contains_key(NOTICE_CONTEXT_KEY));
    assert_eq!(saved_outbox(&f.record()).unwrap(), original);
    let revision = f.record().revision;
    assert!(block_on(f.parent.clear_delivery(&f.session, &delivery)).is_err());
    assert_eq!(f.record().revision, revision);
    let id = delivery.originals()[0].identity();
    assert!(
        f.parent
            .confirm_source_acknowledgements(&delivery, &[id.clone(), id])
            .is_err()
    );
    let foreign = Fixture::new();
    assert!(
        foreign
            .parent
            .confirm_source_acknowledgements(&delivery, &[])
            .is_err()
    );
    acknowledge(&f.parent, &delivery);
    block_on(f.parent.clear_delivery(&f.session, &delivery)).unwrap();
    assert!(saved_outbox(&f.record()).unwrap().is_none());
    assert!(f.parent.delivery().is_none());
    assert!(
        f.parent
            .confirm_source_acknowledgements(&delivery, &[])
            .is_err()
    );
    assert!(block_on(f.parent.clear_delivery(&f.session, &delivery)).is_err());
}

#[test]
fn restart_readback_is_inert_until_unchanged_metadata_save_confirms_original() {
    let store = InMemorySessionStore::default();
    let f = Fixture::with_store(store.clone());
    publish(&f);
    ordinary(&f);
    let original = f.parent.delivery().unwrap();
    let engine = Engine::builder()
        .provider(ScriptedModelProvider::new("test", []))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .session_store(store)
        .build()
        .unwrap();
    let session = block_on(engine.load_session(f.session.id().clone()))
        .unwrap()
        .unwrap();
    let parent = ParentNoticeContext::new(&session, principal("parent"), &f.notices);
    let before = session.record();
    assert!(saved_outbox(&before).unwrap().is_some());
    assert!(parent.delivery().is_none());
    assert!(
        parent
            .prepare(&session, &before, checkpoint(&before), None, None)
            .unwrap()
            .is_none()
    );
    let receipt = block_on(parent.recover_delivery(&session))
        .unwrap()
        .unwrap();
    assert_eq!(
        receipt.provenance(),
        NoticeDeliveryProvenance::RecoveredOriginal
    );
    assert_eq!(receipt.originals(), original.originals());
    assert_eq!(receipt.checkpoint(), original.checkpoint());
    let after = session.record();
    assert!(after.revision > before.revision);
    assert_eq!(after.messages, before.messages);
    assert_eq!(after.metadata, before.metadata);
    assert_eq!(after.next_turn_sequence, before.next_turn_sequence);
    acknowledge(&parent, &receipt);
    block_on(parent.clear_delivery(&session, &receipt)).unwrap();
    assert!(saved_outbox(&session.record()).unwrap().is_none());
}

struct ControlledStore {
    store: InMemorySessionStore,
    mode: Arc<AtomicUsize>,
}
impl machine_god_core::SessionStore for ControlledStore {
    fn load(
        &self,
        id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, machine_god_core::SessionStoreError>> {
        machine_god_core::SessionStore::load(&self.store, id)
    }
    fn save(
        &self,
        record: SessionRecord,
        revision: Option<machine_god_core::SessionRevision>,
    ) -> BoxFuture<'_, Result<machine_god_core::SessionRevision, machine_god_core::SessionStoreError>>
    {
        Box::pin(async move {
            let result =
                machine_god_core::SessionStore::save(&self.store, record, revision).await?;
            match self.mode.swap(0, Ordering::SeqCst) {
                1 => {
                    return Err(machine_god_core::SessionStoreError::new(
                        machine_god_core::SessionStoreErrorKind::Unavailable,
                        "ambiguous",
                        "committed before error",
                        false,
                    ));
                }
                2 => std::future::pending::<()>().await,
                _ => {}
            }
            Ok(result)
        })
    }
}
fn repair_failure(mode: usize) {
    let control = Arc::new(AtomicUsize::new(0));
    let f = Fixture::with_store(ControlledStore {
        store: InMemorySessionStore::default(),
        mode: Arc::clone(&control),
    });
    publish(&f);
    let parent = ParentNoticeContext::new(&f.session, principal("parent"), &f.notices);
    control.store(mode, Ordering::SeqCst);
    let mut future = Box::pin(parent.recover_delivery(&f.session));
    let polled = future
        .as_mut()
        .poll(&mut Context::from_waker(noop_waker_ref()));
    if mode == 1 {
        assert!(matches!(
            polled,
            Poll::Ready(Err(NoticePublicationError::Core(_)))
        ));
    } else {
        assert!(polled.is_pending());
    }
    drop(future);
    assert!(parent.delivery().is_none());
    block_on(f.engine.load_session(f.session.id().clone()))
        .unwrap()
        .unwrap();
    assert!(saved_outbox(&f.record()).unwrap().is_some());
    assert!(parent.delivery().is_none());
    assert!(
        parent
            .prepare(&f.session, &f.record(), f.checkpoint(), None, None)
            .unwrap()
            .is_none()
    );
    let before = f.record();
    let delivery = block_on(parent.recover_delivery(&f.session))
        .unwrap()
        .unwrap();
    assert_eq!(
        delivery.provenance(),
        NoticeDeliveryProvenance::RecoveredOriginal
    );
    assert!(f.record().revision > before.revision);
}
#[test]
fn recovery_committed_error_keeps_fence_until_explicit_confirmed_repair() {
    repair_failure(1);
}
#[test]
fn recovery_committed_pending_drop_keeps_fence_until_explicit_confirmed_repair() {
    repair_failure(2);
}

fn clear_failure(mode: usize) {
    let control = Arc::new(AtomicUsize::new(0));
    let f = Fixture::with_store(ControlledStore {
        store: InMemorySessionStore::default(),
        mode: Arc::clone(&control),
    });
    publish(&f);
    let delivery = f.parent.delivery().unwrap();
    acknowledge(&f.parent, &delivery);
    control.store(mode, Ordering::SeqCst);
    let mut future = Box::pin(f.parent.clear_delivery(&f.session, &delivery));
    let polled = future
        .as_mut()
        .poll(&mut Context::from_waker(noop_waker_ref()));
    if mode == 1 {
        assert!(matches!(
            polled,
            Poll::Ready(Err(NoticePublicationError::Core(_)))
        ));
    } else {
        assert!(polled.is_pending());
    }
    drop(future);
    block_on(f.engine.load_session(f.session.id().clone()))
        .unwrap()
        .unwrap();
    assert!(saved_outbox(&f.record()).unwrap().is_none());
    assert!(f.parent.delivery().is_some());
    ordinary(&f); // A readback or unrelated save does not silently settle the clear.
    assert!(f.parent.delivery().is_some());
    let before = f.record().revision;
    block_on(f.parent.clear_delivery(&f.session, &delivery)).unwrap();
    assert!(f.record().revision > before);
    assert!(f.parent.delivery().is_none());
}
#[test]
fn clear_committed_error_requires_explicit_confirmation_even_when_key_is_absent() {
    clear_failure(1);
}
#[test]
fn clear_committed_pending_drop_requires_explicit_confirmation_even_when_key_is_absent() {
    clear_failure(2);
}

#[test]
fn recovery_rejects_foreign_parent_and_outbox_limits_before_publication() {
    let f = Fixture::new();
    publish(&f);
    let foreign = ParentNoticeContext::new(&f.session, principal("other"), &f.notices);
    let revision = f.record().revision;
    assert!(block_on(foreign.recover_delivery(&f.session)).is_err());
    assert_eq!(f.record().revision, revision);
    let mut record = f.record();
    record.metadata.insert(
        NOTICE_OUTBOX_KEY.into(),
        serde_json::Value::String("x".repeat(65536)),
    );
    assert!(matches!(
        saved_outbox(&record),
        Err(NoticeContextError::ResourceLimit)
    ));
    let mut value = serde_json::Value::Null;
    for _ in 0..32 {
        value = serde_json::Value::Array(vec![value]);
    }
    record.metadata.insert(NOTICE_OUTBOX_KEY.into(), value);
    assert!(matches!(
        saved_outbox(&record),
        Err(NoticeContextError::ResourceLimit)
    ));
    let mut record = f.record();
    let original = record.metadata[NOTICE_OUTBOX_KEY]["originals"][0].clone();
    record.metadata.get_mut(NOTICE_OUTBOX_KEY).unwrap()["originals"] =
        serde_json::Value::Array(vec![original; 65]);
    assert!(saved_outbox(&record).is_err());
}

#[test]
fn delivery_receipt_does_not_retain_session_runtime_and_retirement_fences_it() {
    let f = Fixture::new();
    publish(&f);
    let receipt = f.parent.delivery().unwrap();
    let witness = f.session.witness();
    f.parent.retire();
    assert!(f.parent.delivery().is_none());
    assert!(
        f.parent
            .confirm_source_acknowledgements(&receipt, &[])
            .is_err()
    );
    drop(f);
    assert!(!witness.is_live());
    assert_eq!(receipt.originals().len(), 1);
}

#[test]
fn full_batch_subset_ack_is_atomic_and_requires_the_last_exact_original() {
    let f = Fixture::new();
    for index in 1..64 {
        let work = f
            .notices
            .register_work(
                &WorkNoticeIdentity {
                    source: principal(&format!("child-{index}")),
                    work_id: "work".into(),
                    work_generation: NonZeroU64::new(1).unwrap(),
                },
                ManagedNotifications::default(),
                &NoticeRelationship {
                    generation: NonZeroU64::new(1).unwrap(),
                    parent: Some(principal("parent")),
                },
                0,
            )
            .unwrap();
        let PreparedNotice::Staged(stage) = f
            .notices
            .prepare_terminal(
                &work,
                NonZeroU64::new(1).unwrap(),
                NoticeTerminal::Completed,
                None,
            )
            .unwrap()
        else {
            panic!("staged terminal");
        };
        f.notices.confirm_durable(&stage).unwrap();
    }
    publish(&f);
    let delivery = f.parent.delivery().unwrap();
    assert_eq!(delivery.originals().len(), 64);
    let ids = delivery
        .originals()
        .iter()
        .map(ManagedNotice::identity)
        .collect::<Vec<_>>();
    let mut forged = ids[0].clone();
    forged.source.source.id = "foreign-child".into();
    assert!(
        f.parent
            .confirm_source_acknowledgements(&delivery, &[ids[63].clone(), forged])
            .is_err()
    );
    f.parent
        .confirm_source_acknowledgements(&delivery, &ids[..63])
        .unwrap();
    assert!(block_on(f.parent.clear_delivery(&f.session, &delivery)).is_err());
    f.parent
        .confirm_source_acknowledgements(&delivery, &ids[63..])
        .unwrap();
    block_on(f.parent.clear_delivery(&f.session, &delivery)).unwrap();
    assert!(f.parent.delivery().is_none());
}
