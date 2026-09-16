use super::*;

#[test]
fn same_transcript_replacement_transfers_actual_prompt_registration_without_rebinding_old_one() {
    run(async {
        let inbox = crate::NativeInteractivePromptInbox::new(
            crate::NativeInteractivePromptLimits::default(),
        )
        .unwrap();
        let factory = Arc::new(Factory::new());
        factory.select_managed_inbox(&inbox);
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
        let principal = owner.current().unwrap().principal();
        let host = Arc::downgrade(owner.current_host().unwrap());
        for selection in [
            NativeAcpSessionSelection::Load(principal.session_id().clone()),
            NativeAcpSessionSelection::Resume(principal.session_id().clone()),
        ] {
            let original = inbox.registration_for_owner(&principal).unwrap();
            let runtime = Arc::downgrade(owner.current().unwrap().runtime());
            let mcp = Arc::downgrade(
                owner
                    .current()
                    .unwrap()
                    .command_services
                    .mcp
                    .as_ref()
                    .unwrap(),
            );
            owner
                .request(selection, factory.workspace.clone(), empty(), 2)
                .unwrap();
            assert!(matches!(
                outcome(&mut owner).await,
                NativeAcpSelectionOutcome::Selected { .. }
            ));
            assert_eq!(owner.current().unwrap().principal(), principal);
            assert!(host.ptr_eq(&Arc::downgrade(owner.current_host().unwrap())));
            assert!(!runtime.ptr_eq(&Arc::downgrade(owner.current().unwrap().runtime())));
            assert!(
                !mcp.ptr_eq(&Arc::downgrade(
                    owner
                        .current()
                        .unwrap()
                        .command_services
                        .mcp
                        .as_ref()
                        .unwrap()
                ))
            );
            assert!(!original.is_live());
            assert!(inbox.registration_for_owner(&principal).unwrap().is_live());
            let _ = owner.current_mut().unwrap().take_loaded_history();
        }
        let original = inbox.registration_for_owner(&principal).unwrap();
        owner
            .request(
                NativeAcpSessionSelection::Resume(SessionId::new("missing-handover").unwrap()),
                factory.workspace.clone(),
                empty(),
                3,
            )
            .unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Rejected {
                old_preserved: true,
                ..
            }
        ));
        assert!(original.is_live());
        assert_eq!(factory.preparations.load(Ordering::Acquire), 1);
        close(&mut owner).await;
        assert!(!original.is_live());
        assert!(inbox.registration_for_owner(&principal).is_none());
    });
}

async fn opened() -> (Arc<Factory>, NativeAcpSelectionOwner) {
    let factory = Arc::new(Factory::new());
    factory.managed.store(true, Ordering::Release);
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
    (factory, owner)
}
async fn close(owner: &mut NativeAcpSelectionOwner) {
    owner.request_close(&owner.current().unwrap().id()).unwrap();
    assert!(matches!(
        outcome(owner).await,
        NativeAcpSelectionOutcome::Closed { .. }
    ));
}

#[test]
fn managed_replacement_reuses_host_and_load_alone_replays_history() {
    run(async {
        let (factory, mut owner) = opened().await;
        let host = Arc::downgrade(owner.current_host().unwrap());
        let first = owner.current().unwrap().id();
        let original = owner.current().unwrap().principal();
        for (selection, replay) in [
            (NativeAcpSessionSelection::New, false),
            (NativeAcpSessionSelection::Load(first.clone()), true),
            (NativeAcpSessionSelection::Resume(first), false),
        ] {
            let previous_runtime = Arc::downgrade(owner.current().unwrap().runtime());
            owner
                .request(selection, factory.workspace.clone(), empty(), 2)
                .unwrap();
            let selected = outcome(&mut owner).await;
            match &selected {
                NativeAcpSelectionOutcome::Rejected { error, .. }
                | NativeAcpSelectionOutcome::Indeterminate { error, .. } => {
                    panic!("managed replacement failed (load={replay}): {error:?}");
                }
                _ => {}
            }
            assert!(matches!(
                selected,
                NativeAcpSelectionOutcome::Selected { .. }
            ));
            assert!(host.ptr_eq(&Arc::downgrade(owner.current_host().unwrap())));
            assert!(!previous_runtime.ptr_eq(&Arc::downgrade(owner.current().unwrap().runtime())));
            assert_eq!(
                owner.current_mut().unwrap().take_loaded_history().is_some(),
                replay
            );
        }
        // Resume preserves the durable transcript incarnation, while the exact
        // runtime allocation and its ephemeral MCP selection must be replaced.
        assert_eq!(owner.current().unwrap().principal(), original);
        assert_eq!(factory.preparations.load(Ordering::Acquire), 1);
        assert!(!factory.provider_started.load(Ordering::Acquire));
        close(&mut owner).await;
    });
}

#[test]
fn failed_reuse_resume_preserves_old_parent_and_allows_explicit_retry() {
    run(async {
        let (factory, mut owner) = opened().await;
        let previous = owner.current().unwrap().principal();
        owner
            .request(
                NativeAcpSessionSelection::Resume(SessionId::new("missing-reused-parent").unwrap()),
                factory.workspace.clone(),
                empty(),
                2,
            )
            .unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Rejected {
                old_preserved: true,
                ..
            }
        ));
        assert_eq!(owner.current().unwrap().principal(), previous);
        assert!(!owner.is_fenced());
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
        assert_eq!(factory.preparations.load(Ordering::Acquire), 1);
        close(&mut owner).await;
    });
}

#[test]
fn failed_reuse_readiness_preserves_the_old_mcp_instance() {
    run(async {
        let (factory, mut owner) = opened().await;
        let previous = owner.current().unwrap().principal();
        let old_mcp = owner
            .current()
            .unwrap()
            .command_services
            .mcp
            .clone()
            .unwrap();
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
        assert!(Arc::ptr_eq(
            &old_mcp,
            owner
                .current()
                .unwrap()
                .command_services
                .mcp
                .as_ref()
                .unwrap()
        ));
        drop(old_mcp);
        close(&mut owner).await;
    });
}

#[test]
fn cancellation_during_reuse_check_does_not_close_the_current_host() {
    run(async {
        let (factory, mut owner) = opened().await;
        let previous = owner.current().unwrap().principal();
        factory.wait.store(true, Ordering::Release);
        owner
            .request(
                NativeAcpSessionSelection::New,
                factory.workspace.clone(),
                empty(),
                2,
            )
            .unwrap();
        let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
        let _ = owner.poll_progress(&mut cx, 2);
        assert!(owner.cancel_pending());
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Rejected {
                error: AcpSessionError::Cancelled,
                old_preserved: true,
                ..
            }
        ));
        assert!(factory.cancel_observed.load(Ordering::Acquire));
        assert_eq!(owner.current().unwrap().principal(), previous);
        close(&mut owner).await;
    });
}

#[test]
fn cancellation_of_ready_reuse_stage_releases_ticket_without_replacing_parent() {
    run(async {
        let (factory, mut owner) = opened().await;
        let previous = owner.current().unwrap().principal();
        owner
            .request(
                NativeAcpSessionSelection::New,
                factory.workspace.clone(),
                empty(),
                2,
            )
            .unwrap();
        futures_util::future::poll_fn(|cx| {
            let _ = owner.current.as_mut().unwrap().session.poll_progress(cx, 2);
            let pending = owner.pending.take().unwrap();
            let _ = owner.advance(pending, cx);
            if matches!(
                owner.pending.as_ref().map(|pending| &pending.phase),
                Some(Phase::ReuseDraining(_))
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
                old_preserved: true,
                candidate_may_have_persisted: false,
                ..
            }
        ));
        assert_eq!(owner.current().unwrap().principal(), previous);
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
        assert_eq!(factory.preparations.load(Ordering::Acquire), 1);
        close(&mut owner).await;
    });
}

#[test]
fn reuse_checks_actual_descriptors_and_never_rebinds_a_token_to_another_host() {
    run(async {
        let factory = Factory::new();
        factory.managed.store(true, Ordering::Release);
        let managed = factory
            .prepare(
                factory.workspace.clone(),
                NativeMcpNetworkRequirement::None,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let environment = crate::NativeEnvironment::new(
            None,
            Some(
                factory
                    .workspace
                    .parent()
                    .unwrap()
                    .join("state")
                    .into_os_string(),
            ),
            None,
        );
        let roots = || {
            crate::PreparedNativeRoots::prepare(
                crate::NativeRootSelection::from_environment(&environment, &factory.workspace)
                    .unwrap(),
            )
            .unwrap()
        };
        let checked = NativeAcpHostReuse::capture(&managed.host, &roots())
            .unwrap()
            .unwrap();
        assert!(checked.matches(&managed.host, &factory.workspace));
        factory.managed.store(false, Ordering::Release);
        let other = factory
            .prepare(
                factory.workspace.clone(),
                NativeMcpNetworkRequirement::None,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(!checked.matches(&other.host, &factory.workspace));
        assert!(cleanup::reject(other, None, 1).await.complete);
        let original = factory.workspace.with_extension("original");
        std::fs::rename(&factory.workspace, &original).unwrap();
        std::fs::create_dir(&factory.workspace).unwrap();
        assert!(
            NativeAcpHostReuse::capture(&managed.host, &roots())
                .unwrap()
                .is_none()
        );
        // The previously checked token retains the original authority, not the
        // newly created object which now has the same pathname.
        assert!(checked.matches(&managed.host, &factory.workspace));
        drop(checked);
        assert!(cleanup::reject(managed, None, 2).await.complete);
    });
}

#[test]
fn failed_reuse_readiness_does_not_cancel_a_running_old_prompt() {
    run(async {
        let (factory, mut owner) = opened().await;
        let id = owner.current().unwrap().id();
        let prompt = crate::acp::prompt::decode_prompt_input(
            &serde_json::json!({"prompt":[{"type":"text","text":"retain this prompt"}]}),
        )
        .unwrap();
        owner.current_mut().unwrap().enqueue(&id, prompt).unwrap();
        futures_util::future::poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 2);
            let _ = owner.take_presentation();
            if factory.provider_started.load(Ordering::Acquire) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
        let missing = NativeMcpEphemeralConfiguration::decode(Some(
            br#"[{"name":"missing","command":"/bin/sh","args":[],"env":[]}]"#,
        ))
        .unwrap();
        owner
            .request(
                NativeAcpSessionSelection::New,
                factory.workspace.clone(),
                missing,
                3,
            )
            .unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Rejected {
                old_preserved: true,
                ..
            }
        ));
        assert!(owner.current().unwrap().has_pending_prompt());
        assert!(!owner.current().unwrap().cancellation_requested());
        assert!(owner.request_cancel(&id).unwrap());
        futures_util::future::poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 4);
            let _ = owner.take_presentation();
            owner.take_turn_outcome().map_or(Poll::Pending, Poll::Ready)
        })
        .await;
        close(&mut owner).await;
    });
}
