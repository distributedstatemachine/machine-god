use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct RejectFactory(AtomicUsize);
impl NativeAcpHostFactory for RejectFactory {
    fn prepare(
        &self,
        _: PathBuf,
        _: NativeMcpNetworkRequirement,
        _: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeAcpPreparedHost, AcpSessionError>> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Box::pin(async { Err(AcpSessionError::Unavailable) })
    }
    fn list(
        &self,
        _: Option<PathBuf>,
        _: Option<NativeSessionCatalogCursor>,
        _: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeSessionCatalogPage, NativeSessionCatalogReadError>> {
        Box::pin(async { Err(NativeSessionCatalogReadError::Unavailable) })
    }
}
fn empty() -> NativeMcpEphemeralConfiguration {
    NativeMcpEphemeralConfiguration::decode(None).unwrap()
}

/// Composition fixtures supply an actual already-opened native owner. No
/// factory effect or public ID can manufacture its principal or service routes.
pub(crate) fn preopened(
    session: NativeAcpSession,
    host: Arc<NativeReferenceHost>,
) -> NativeAcpSelectionOwner {
    let permission_contexts = host.permission_contexts().unwrap();
    let mut owner = NativeAcpSelectionOwner::new(Arc::new(RejectFactory(AtomicUsize::new(0))));
    owner.current = Some(Current {
        session,
        host,
        permission_contexts,
    });
    owner
}

pub(crate) fn retains_turn_outcome(owner: &NativeAcpSelectionOwner) -> bool {
    owner.turn_outcome.is_some()
}
fn run(future: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(20), future)
                .await
                .expect("owned selection must settle");
        });
}
async fn outcome(owner: &mut NativeAcpSelectionOwner) -> NativeAcpSelectionOutcome {
    let outcome = futures_util::future::poll_fn(|cx| {
        let _ = owner.poll_progress(cx, 200);
        owner.take_outcome().map_or(Poll::Pending, Poll::Ready)
    })
    .await;
    match &outcome {
        NativeAcpSelectionOutcome::Rejected {
            error,
            old_preserved,
            candidate_may_have_persisted,
            ..
        } => eprintln!(
            "selection rejected: {error:?}, preserved={old_preserved}, persisted={candidate_may_have_persisted}"
        ),
        NativeAcpSelectionOutcome::Indeterminate { error, .. } => {
            eprintln!("selection indeterminate: {error:?}");
        }
        _ => {}
    }
    outcome
}

#[test]
fn selection_requests_are_inert_and_reject_busy_or_invalid_intent() {
    let factory = Arc::new(RejectFactory(AtomicUsize::new(0)));
    let mut owner = NativeAcpSelectionOwner::new(factory.clone());
    assert!(
        owner
            .request(
                NativeAcpSessionSelection::New,
                "relative".into(),
                empty(),
                1
            )
            .is_err()
    );
    owner
        .request(
            NativeAcpSessionSelection::New,
            "/workspace".into(),
            empty(),
            1,
        )
        .unwrap();
    assert!(matches!(
        owner.request(
            NativeAcpSessionSelection::New,
            "/workspace".into(),
            empty(),
            1
        ),
        Err(AcpSessionError::Busy)
    ));
    assert_eq!(factory.0.load(Ordering::Relaxed), 0);
    drop(owner);
    assert_eq!(factory.0.load(Ordering::Relaxed), 0);
}

#[test]
fn cancelled_unpolled_selection_never_calls_factory_or_claims_publication() {
    run(async {
        let factory = Arc::new(RejectFactory(AtomicUsize::new(0)));
        let mut owner = NativeAcpSelectionOwner::new(factory.clone());
        let id = owner
            .request(
                NativeAcpSessionSelection::New,
                "/workspace".into(),
                empty(),
                1,
            )
            .unwrap();
        assert!(owner.cancel_pending());
        assert!(
            matches!(outcome(&mut owner).await,NativeAcpSelectionOutcome::Rejected{id:actual,error:AcpSessionError::Cancelled,candidate_may_have_persisted:false,..}if actual==id)
        );
        assert_eq!(factory.0.load(Ordering::Relaxed), 0);
        owner.request_shutdown();
        assert!(owner.is_closed());
    });
}

mod cleanup_wait;
pub(crate) mod fixture;
mod managed;
mod network_requirements;
mod reuse;
mod workspace_identity;
use fixture::Factory;

#[test]
fn selected_cancellation_during_preparation_does_not_cancel_the_candidate() {
    run(async {
        let factory = Arc::new(Factory::new());
        let mut owner = NativeAcpSelectionOwner::new(factory.clone());
        owner
            .request(
                NativeAcpSessionSelection::New,
                factory.workspace.clone(),
                empty(),
                1,
            )
            .unwrap();
        let _ = outcome(&mut owner).await;
        let selected = owner.current().unwrap().id();
        factory.wait.store(true, Ordering::Release);
        owner
            .request(
                NativeAcpSessionSelection::New,
                factory.workspace.clone(),
                empty(),
                2,
            )
            .unwrap();
        futures_util::future::poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 2);
            Poll::Ready(())
        })
        .await;
        assert!(owner.current_mut().is_none());
        assert!(!owner.request_cancel(&selected).unwrap());
        assert!(!factory.cancel_observed.load(Ordering::Acquire));
        assert!(matches!(
            owner.request_cancel(&SessionId::new("foreign").unwrap()),
            Err(AcpSessionError::WrongSession)
        ));
        assert!(owner.cancel_pending());
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Rejected {
                old_preserved: true,
                ..
            }
        ));
        assert!(factory.cancel_observed.load(Ordering::Acquire));
        owner.request_close(&selected).unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Closed { .. }
        ));
    });
}

#[test]
fn resume_replaces_owned_host_without_replaying_history() {
    run(async {
        let factory = Arc::new(Factory::new());
        let mut owner = NativeAcpSelectionOwner::new(factory.clone());
        owner
            .request(
                NativeAcpSessionSelection::New,
                factory.workspace.clone(),
                empty(),
                1,
            )
            .unwrap();
        let _ = outcome(&mut owner).await;
        let id = owner.current().unwrap().id();
        let old_host = Arc::downgrade(owner.current_host().unwrap());
        assert!(!owner.current_host().unwrap().managed_agents_selected());
        owner
            .request(
                NativeAcpSessionSelection::Resume(id.clone()),
                factory.workspace.clone(),
                empty(),
                2,
            )
            .unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Selected {
                previous: Some(_),
                ..
            }
        ));
        assert!(old_host.upgrade().is_none());
        assert_eq!(factory.preparations.load(Ordering::Acquire), 2);
        assert!(owner.current_mut().unwrap().take_loaded_history().is_none());
        assert_eq!(owner.current().unwrap().id(), id);
        owner.request_shutdown();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Closed { .. }
        ));
    });
}

#[test]
fn shared_candidate_mcp_contexts_are_rejected_before_activation() {
    run(async {
        let factory = Arc::new(Factory::new());
        let mut owner = NativeAcpSelectionOwner::new(factory.clone());
        owner
            .request(
                NativeAcpSessionSelection::New,
                factory.workspace.clone(),
                empty(),
                1,
            )
            .unwrap();
        let _ = outcome(&mut owner).await;
        let id = owner.current().unwrap().id();
        *factory.mcp_contexts_override.lock().unwrap() =
            owner.current_host().unwrap().mcp_contexts();
        owner
            .request(
                NativeAcpSessionSelection::New,
                factory.workspace.clone(),
                empty(),
                2,
            )
            .unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Rejected {
                error: AcpSessionError::InvalidConfiguration,
                old_preserved: true,
                candidate_may_have_persisted: false,
                ..
            }
        ));
        assert!(
            owner
                .current_host()
                .unwrap()
                .mcp_ephemeral_owner()
                .unwrap()
                .ready()
                .is_ok()
        );
        owner.request_close(&id).unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Closed { .. }
        ));
    });
}

#[test]
fn rejected_or_cancelled_same_id_rebind_never_claims_the_old_checkpoint_is_preserved() {
    for cancel in [false, true] {
        run(async {
            let factory = Arc::new(Factory::new());
            let mut owner = NativeAcpSelectionOwner::new(factory.clone());
            owner
                .request(
                    NativeAcpSessionSelection::New,
                    factory.workspace.clone(),
                    empty(),
                    1,
                )
                .unwrap();
            let _ = outcome(&mut owner).await;
            let id = owner.current().unwrap().id();
            let original = owner.current().unwrap().runtime().record_snapshot();
            let other = factory.workspace.parent().unwrap().join("other-workspace");
            std::fs::create_dir(&other).unwrap();
            owner.after_open = Some(Box::new(move |host, cancellation| {
                if cancel {
                    cancellation.cancel();
                } else {
                    host.close_mcp();
                }
            }));
            owner
                .request(
                    NativeAcpSessionSelection::Load(id.clone()),
                    other,
                    empty(),
                    2,
                )
                .unwrap();
            assert!(matches!(
                outcome(&mut owner).await,
                NativeAcpSelectionOutcome::Indeterminate { .. }
            ));
            assert!(owner.is_fenced());
            assert!(owner.current_mut().is_none());
            assert!(
                owner
                    .current()
                    .unwrap()
                    .runtime()
                    .begin_quiescence()
                    .is_err(),
                "the old runtime remains fenced after failed rollback proof"
            );
            let store = owner.current_host().unwrap().session_store().clone();
            let actual = machine_god_core::SessionStore::load(&*store, id)
                .await
                .unwrap()
                .unwrap();
            assert_ne!(actual.revision, original.revision);
            assert_eq!(actual.incarnation_id, original.incarnation_id);
            owner.request_shutdown();
            assert!(matches!(
                outcome(&mut owner).await,
                NativeAcpSelectionOutcome::Indeterminate { .. }
            ));
            assert!(owner.is_closed());
        });
    }
}

#[test]
fn successful_same_id_rebind_retires_old_without_overwriting_the_new_checkpoint() {
    run(async {
        let factory = Arc::new(Factory::new());
        let mut owner = NativeAcpSelectionOwner::new(factory.clone());
        owner
            .request(
                NativeAcpSessionSelection::New,
                factory.workspace.clone(),
                empty(),
                1,
            )
            .unwrap();
        let _ = outcome(&mut owner).await;
        let id = owner.current().unwrap().id();
        let original = owner.current().unwrap().runtime().record_snapshot();
        let old_host = Arc::downgrade(owner.current_host().unwrap());
        let other = factory.workspace.parent().unwrap().join("other-workspace");
        std::fs::create_dir(&other).unwrap();
        owner
            .request(
                NativeAcpSessionSelection::Load(id.clone()),
                other.clone(),
                empty(),
                2,
            )
            .unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Selected { .. }
        ));
        assert!(old_host.upgrade().is_none());
        let adopted = owner.current().unwrap().runtime().record_snapshot();
        assert_ne!(adopted.revision, original.revision);
        assert_eq!(
            crate::NativeSessionMetadata::from_metadata(&adopted.metadata)
                .unwrap()
                .workspace(),
            Some(other.as_path())
        );
        let prompt = super::super::prompt::decode_prompt_input(
            &serde_json::json!({"prompt":[{"type":"text","text":"checkpoint after rebind"}]}),
        )
        .unwrap();
        owner.current_mut().unwrap().enqueue(&id, prompt).unwrap();
        futures_util::future::poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 300);
            let _ = owner.take_presentation();
            if factory.provider_started.load(Ordering::Acquire) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
        owner.request_cancel(&id).unwrap();
        let turn = futures_util::future::poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 300);
            let _ = owner.take_presentation();
            owner.take_turn_outcome().map_or(Poll::Pending, Poll::Ready)
        })
        .await;
        assert!(turn.outcome.is_ok(), "{:?}", turn.outcome);
        let saved = owner.current().unwrap().runtime().record_snapshot();
        assert_ne!(saved.revision, adopted.revision);
        assert!(saved.messages.iter().any(|message| message.content.iter().any(|block| matches!(block, machine_god_core::ContentBlock::Text { text } if text == "checkpoint after rebind"))));
        let store = owner.current_host().unwrap().session_store().clone();
        let durable = machine_god_core::SessionStore::load(&*store, id.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(durable.revision, saved.revision);
        assert_eq!(
            crate::NativeSessionMetadata::from_metadata(&durable.metadata)
                .unwrap()
                .workspace(),
            Some(other.as_path())
        );
        owner.request_close(&id).unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Closed { .. }
        ));
    });
}

#[test]
fn readiness_and_native_validation_failures_preserve_the_previous_selection() {
    run(async {
        let factory = Arc::new(Factory::new());
        let mut owner = NativeAcpSelectionOwner::new(factory.clone());
        owner
            .request(
                NativeAcpSessionSelection::New,
                factory.workspace.clone(),
                empty(),
                1,
            )
            .unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Selected { previous: None, .. }
        ));
        let original = owner.current().unwrap().principal();
        let original_host = Arc::downgrade(owner.current_host().unwrap());
        let unavailable = NativeMcpEphemeralConfiguration::decode(Some(
            br#"[{"name":"unavailable","command":"/no-selected-authority","args":[],"env":[]}]"#,
        ))
        .unwrap();
        owner
            .request(
                NativeAcpSessionSelection::New,
                factory.workspace.clone(),
                unavailable,
                2,
            )
            .unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Rejected {
                old_preserved: true,
                candidate_may_have_persisted: false,
                ..
            }
        ));
        assert_eq!(owner.current().unwrap().principal(), original);
        assert!(Arc::ptr_eq(
            &original_host.upgrade().unwrap(),
            owner.current_host().unwrap()
        ));
        owner
            .request(
                NativeAcpSessionSelection::Load(SessionId::new("missing").unwrap()),
                factory.workspace.clone(),
                empty(),
                3,
            )
            .unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Rejected {
                old_preserved: true,
                candidate_may_have_persisted: true,
                ..
            }
        ));
        assert_eq!(owner.current().unwrap().principal(), original);
        assert!(
            owner
                .current()
                .unwrap()
                .runtime()
                .begin_quiescence()
                .is_ok()
        );
        let id = owner.current().unwrap().id();
        owner.request_close(&id).unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Closed { .. }
        ));
        assert!(original_host.upgrade().is_none());
        assert!(owner.current().is_none());
    });
}

#[test]
fn same_id_load_waits_for_cancelled_prompt_checkpoint_and_retains_its_exact_outcome() {
    run(async {
        let factory = Arc::new(Factory::new());
        let mut owner = NativeAcpSelectionOwner::new(factory.clone());
        owner
            .request(
                NativeAcpSessionSelection::New,
                factory.workspace.clone(),
                empty(),
                1,
            )
            .unwrap();
        let _ = outcome(&mut owner).await;
        let original = owner.current().unwrap().principal();
        let id = owner.current().unwrap().id();
        let prompt = super::super::prompt::decode_prompt_input(
            &serde_json::json!({"prompt":[{"type":"text","text":"persisted before reload"}]}),
        )
        .unwrap();
        owner.current_mut().unwrap().enqueue(&id, prompt).unwrap();
        futures_util::future::poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 1);
            let _ = owner.take_presentation();
            if factory.provider_started.load(Ordering::Acquire) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
        owner
            .request(
                NativeAcpSessionSelection::Load(id.clone()),
                factory.workspace.clone(),
                empty(),
                2,
            )
            .unwrap();
        let turn = futures_util::future::poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 2);
            while owner.take_presentation().is_some() {}
            assert!(
                owner.take_outcome().is_none(),
                "replacement cannot precede old completion drain"
            );
            owner.take_turn_outcome().map_or(Poll::Pending, Poll::Ready)
        })
        .await;
        assert_eq!(turn.owner, original);
        assert!(turn.outcome.is_ok(), "{:?}", turn.outcome);
        assert!(
            matches!(outcome(&mut owner).await,NativeAcpSelectionOutcome::Selected{previous:Some(previous),current,..}if previous==original&&current.session_id()==&id)
        );
        let saved = owner.current().unwrap().runtime().record_snapshot();
        assert!(saved.messages.iter().any(|message|message.content.iter().any(|block|matches!(block,machine_god_core::ContentBlock::Text{text}if text=="persisted before reload"))));
        let history = owner.current_mut().unwrap().take_loaded_history().unwrap();
        drop(history);
        owner.request_shutdown();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Closed { .. }
        ));
        assert!(owner.is_closed());
    });
}

#[test]
fn eof_during_pending_factory_keeps_the_factory_future_owned_until_it_finishes() {
    run(async {
        let factory = Arc::new(Factory::new());
        factory.wait.store(true, Ordering::Release);
        let mut owner = NativeAcpSelectionOwner::new(factory.clone());
        owner
            .request(
                NativeAcpSessionSelection::New,
                factory.workspace.clone(),
                empty(),
                1,
            )
            .unwrap();
        futures_util::future::poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 1);
            Poll::Ready(())
        })
        .await;
        assert_eq!(factory.preparations.load(Ordering::Relaxed), 1);
        owner.request_shutdown();
        assert!(!owner.is_closed());
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Rejected {
                error: AcpSessionError::Cancelled,
                ..
            }
        ));
        assert!(factory.cancel_observed.load(Ordering::Acquire));
        assert!(owner.is_closed());
    });
}
