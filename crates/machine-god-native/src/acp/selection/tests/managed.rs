use super::*;

fn factory() -> Arc<Factory> {
    let factory = Arc::new(Factory::new());
    factory.managed.store(true, Ordering::Release);
    factory
}

async fn close(owner: &mut NativeAcpSelectionOwner) {
    let selected = owner.current().unwrap().id();
    owner.request_close(&selected).unwrap();
    assert!(matches!(
        outcome(owner).await,
        NativeAcpSelectionOutcome::Closed { .. }
    ));
    assert!(owner.current().is_none());
    owner.request_shutdown();
    assert!(owner.is_closed());
}

#[test]
fn managed_selection_adopts_ready_parent_without_a_global_mcp_owner() {
    run(async {
        let factory = factory();
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
        let current = owner.current.as_ref().unwrap();
        assert!(current.host.managed_agents_selected());
        assert!(current.host.mcp_ephemeral_owner().is_none());
        assert!(current.host.mcp_runtime().is_none());
        assert_eq!(
            current
                .host
                .session_lifecycle()
                .list_sessions()
                .await
                .unwrap()
                .session_ids(),
            &[current.session.id()]
        );
        assert!(!factory.provider_started.load(Ordering::Acquire));
        close(&mut owner).await;
        assert_eq!(factory.preparations.load(Ordering::Acquire), 1);
    });
}

#[test]
fn managed_missing_resume_settles_manager_before_explicit_retry() {
    run(async {
        let factory = factory();
        let mut owner = NativeAcpSelectionOwner::new(factory.clone());
        owner
            .request(
                NativeAcpSessionSelection::Resume(SessionId::new("missing-managed-acp").unwrap()),
                factory.workspace.clone(),
                empty(),
                1,
            )
            .unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Rejected {
                old_preserved: false,
                candidate_may_have_persisted: true,
                ..
            }
        ));
        assert!(!owner.fenced);
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
            NativeAcpSelectionOutcome::Selected { .. }
        ));
        assert!(!factory.provider_started.load(Ordering::Acquire));
        close(&mut owner).await;
    });
}

#[test]
fn cancelled_managed_open_keeps_stage_until_cleanup_and_creates_no_transcript() {
    run(async {
        let factory = factory();
        let mut owner = NativeAcpSelectionOwner::new(factory.clone());
        owner
            .request(
                NativeAcpSessionSelection::New,
                factory.workspace.clone(),
                empty(),
                1,
            )
            .unwrap();
        // Advance one phase at a time to stop after admission but before opening
        // is first polled. This exercises custody rather than pre-request cancel.
        futures_util::future::poll_fn(|cx| {
            let pending = owner.pending.take().unwrap();
            let _ = owner.advance(pending, cx);
            if matches!(
                owner.pending.as_ref().map(|p| &p.phase),
                Some(Phase::ManagedOpening { .. })
            ) {
                Poll::Ready(())
            } else {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        })
        .await;
        assert!(owner.cancel_pending());
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Rejected {
                old_preserved: false,
                ..
            }
        ));
        assert!(!owner.fenced);
        // A new owner of the same journal proves the cancelled owner released it.
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
            NativeAcpSelectionOutcome::Selected { .. }
        ));
        let current = owner.current.as_ref().unwrap();
        assert_eq!(
            current
                .host
                .session_lifecycle()
                .list_sessions()
                .await
                .unwrap()
                .session_ids(),
            &[current.session.id()]
        );
        close(&mut owner).await;
    });
}

#[test]
fn failed_managed_peer_readiness_preserves_old_session_and_releases_candidate() {
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
            NativeAcpSelectionOutcome::Selected { .. }
        ));
        let previous = owner.current().unwrap().principal();
        factory.managed.store(true, Ordering::Release);
        let missing = NativeMcpEphemeralConfiguration::decode(Some(
            br#"[{"name":"missing","command":"/bin/sh","args":[],"env":[]}]"#,
        ))
        .unwrap();
        owner
            .request(
                NativeAcpSessionSelection::New,
                factory.workspace.clone(),
                missing,
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
        assert_eq!(owner.current().unwrap().principal(), previous);
        assert!(!owner.fenced);
        owner
            .request(
                NativeAcpSessionSelection::New,
                factory.workspace.clone(),
                empty(),
                3,
            )
            .unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Selected { .. }
        ));
        assert_ne!(owner.current().unwrap().principal(), previous);
        close(&mut owner).await;
    });
}

#[test]
fn retained_managed_cleanup_cannot_report_a_closed_connection() {
    run(async {
        let factory = factory();
        let mut prepared = factory
            .prepare(
                factory.workspace.clone(),
                NativeMcpNetworkRequirement::None,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let mut owner = NativeAcpSelectionOwner::new(factory);
        owner.retained_cleanup.push(cleanup::Receipt {
            complete: false,
            workers: Vec::new(),
            managed: prepared.managed.take(),
        });
        owner.request_shutdown();
        assert!(!owner.is_closed());
        let retained = owner.retained_cleanup.pop().unwrap().managed.unwrap();
        assert!(retained.settle(1).await.is_ok());
        assert!(cleanup::reject(prepared, None, 1).await.complete);
        assert!(owner.is_closed());
    });
}
