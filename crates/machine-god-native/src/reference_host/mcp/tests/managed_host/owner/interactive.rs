use super::*;
mod acp;
mod human;
mod navigation;
mod navigation_forms;
mod navigation_ui;
mod preselection;
mod prompts;
#[cfg(feature = "mcp-http")]
mod startup;
mod workspace;

fn configured_options(
    options: NativeReferenceHostConversationOptions,
    directory: &Directory,
    clock: Arc<Clock>,
) -> NativeReferenceHostConversationOptions {
    let mut options = super::super::options(options, directory, clock.clone());
    let mcp = options
        .mcp_runtime
        .take()
        .unwrap()
        .with_controller_startup(super::super::super::controller::startup(clock));
    options.with_mcp_runtime(mcp).with_mcp_management(Arc::new(
        crate::mcp::management::NativeMcpManagementService::new(Arc::new(
            crate::mcp::store::NativeMcpConfigStore::new(directory.0.join("profile")).unwrap(),
        )),
    ))
}

async fn outcome(owner: &mut NativeInteractiveSession) -> NativeInteractiveOutcome {
    outcome_at(owner, 10).await
}

async fn outcome_at(owner: &mut NativeInteractiveSession, now_ms: i64) -> NativeInteractiveOutcome {
    poll_fn(|cx| {
        let progress = owner.poll_progress(cx, now_ms);
        assert!(
            owner.managed_error().is_none(),
            "{:?}",
            owner.managed_error()
        );
        assert!(
            owner.shutdown_error().is_none(),
            "{:?}",
            owner.shutdown_error()
        );
        let _ = owner.take_presentation();
        if let Some(outcome) = owner.take_outcome() {
            return Poll::Ready(outcome);
        }
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    })
    .await
}

#[test]
fn managed_interactive_transition_retains_child_and_shutdown_waits_for_outer_owner() {
    let mut fixture = Fixture::with_options("auto", true, options);
    let path = journal_path(&fixture);
    fixture.transport.responses.lock().unwrap().extend([
        call(
            "spawn",
            "subagent",
            &serde_json::json!({"command":{"create":{"name":"worker","mode":"persistent"}}}),
        ),
        answer(),
    ]);
    run(async {
        let host = fixture.host.take().unwrap();
        let completion = host.terminal_shutdown_completion().unwrap();
        let options = NativeInteractiveSessionOptions::new(
            fixture.workspace.clone(),
            host.loaded_config().config().model_preferences(),
        )
        .unwrap()
        .with_process_model_override("fixture/process-override")
        .unwrap();
        let mut owner = NativeInteractiveSession::open_managed(
            host,
            directory(&path),
            options,
            NativeInteractiveInitialSession::Fresh,
            1,
        )
        .await
        .unwrap();
        assert_eq!(
            owner.runtime().model_preferences().model(),
            "fixture/process-override"
        );
        owner.enqueue("create a persistent worker".into()).unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeInteractiveOutcome::Turn(Ok(_))
        ));
        assert_eq!(owner.managed_agents().len(), 1);
        let child = owner.managed_agents().remove(0);
        assert_eq!(child.name, "worker");
        let source = owner.runtime().clone();
        owner
            .request_transition(NativeInteractiveTransition::New, 11)
            .unwrap();
        let NativeInteractiveOutcome::Transition(receipt) = outcome(&mut owner).await else {
            panic!("transition receipt");
        };
        assert!(!receipt.unchanged);
        assert_ne!(source.id(), owner.runtime().id());
        assert!(source.enqueue("retired".into()).is_err());
        assert_ne!(
            owner.runtime().model_preferences().model(),
            "fixture/process-override"
        );
        assert_eq!(owner.managed_agents()[0].id, child.id);
        assert_eq!(
            owner.managed_agents()[0].state,
            machine_god_core::ManagedAgentState::Idle
        );
        owner.request_shutdown();
        while !owner.is_closed() {
            let _ = outcome(&mut owner).await;
        }
        assert!(owner.managed_agents().is_empty());
        assert!(!completion.is_complete(), "actual host still owns services");
        drop(source);
        drop(owner);
        completion.wait().await;
        assert!(completion.is_complete());
    });
    assert_eq!(fixture.transport.requests.lock().unwrap().len(), 2);
}

#[test]
fn managed_interactive_open_is_inert_and_idle_shutdown_executes_no_model() {
    let mut fixture = Fixture::with_options("ask", true, options);
    let path = journal_path(&fixture);
    let host = fixture.host.take().unwrap();
    let completion = host.terminal_shutdown_completion().unwrap();
    let options = NativeInteractiveSessionOptions::new(
        fixture.workspace.clone(),
        host.loaded_config().config().model_preferences(),
    )
    .unwrap();
    let future = NativeInteractiveSession::open_managed(
        host,
        directory(&path),
        options,
        NativeInteractiveInitialSession::Fresh,
        1,
    );
    assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
    run(async {
        let mut owner = future.await.unwrap();
        owner
            .request_control(
                NativeInteractiveControl::Mcp {
                    command: crate::mcp::commands::McpCommand::Feature(
                        crate::mcp::commands::McpFeatureCommand::ResourceList {
                            server: "missing".into(),
                        },
                    ),
                },
                2,
            )
            .unwrap();
        let control = poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 3);
            owner
                .take_control_outcome()
                .map_or(Poll::Pending, Poll::Ready)
        })
        .await;
        assert!(
            matches!(
                control.result,
                Err(NativeInteractiveControlError::McpFeature(_))
            ),
            "{control:?}"
        );
        owner.request_shutdown();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeInteractiveOutcome::Shutdown
        ));
        assert!(owner.is_closed());
        drop(owner);
        completion.wait().await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn configured_parent_and_child_startup_use_their_own_admission_and_reach_the_provider() {
    let mut fixture = Fixture::with_options("auto", true, configured_options);
    let path = journal_path(&fixture);
    fixture.transport.responses.lock().unwrap().extend([
        call("spawn", "subagent", &serde_json::json!({"command":{"create":{"name":"worker","mode":"persistent","prompt":"standalone child task"}}})),
        answer(), answer(),
    ]);
    run(async {
        let host = fixture.host.take().unwrap();
        let completion = host.terminal_shutdown_completion().unwrap();
        let options = NativeInteractiveSessionOptions::new(
            fixture.workspace.clone(),
            host.loaded_config().config().model_preferences(),
        )
        .unwrap();
        let mut owner = NativeInteractiveSession::open_managed(
            host,
            directory(&path),
            options,
            NativeInteractiveInitialSession::Fresh,
            1,
        )
        .await
        .unwrap();
        assert!(
            fixture.transport.requests.lock().unwrap().is_empty(),
            "startup alone does not poll a provider"
        );
        owner.enqueue("create the worker".into()).unwrap();
        let NativeInteractiveOutcome::Turn(Ok(event)) = outcome(&mut owner).await else {
            panic!("parent completion");
        };
        assert!(matches!(event.payload, TurnEvent::Completed { .. }));
        poll_fn(|cx| {
            let progress = owner.poll_progress(cx, 20);
            assert!(owner.managed_error().is_none());
            if fixture.transport.requests.lock().unwrap().len() == 3
                && owner
                    .managed_agents()
                    .first()
                    .is_some_and(|child| child.state == machine_god_core::ManagedAgentState::Idle)
            {
                return Poll::Ready(());
            }
            if progress.is_ready() {
                cx.waker().wake_by_ref();
            }
            Poll::Pending
        })
        .await;
        owner.request_shutdown();
        while !owner.is_closed() {
            let _ = outcome(&mut owner).await;
        }
        drop(owner);
        completion.wait().await;
    });
    assert_eq!(fixture.transport.requests.lock().unwrap().len(), 3);
}

#[test]
fn failed_configured_parent_startup_preserves_management_and_explicit_reload_repairs_it() {
    use std::os::unix::fs::PermissionsExt;
    let mut fixture = Fixture::with_options("auto", true, |options, directory, clock| {
        let profile = directory.0.join("profile");
        fs::DirBuilder::new().mode(0o700).create(&profile).unwrap();
        let config = profile.join("mcp.json");
        fs::write(&config, "invalid configuration").unwrap();
        fs::set_permissions(config, fs::Permissions::from_mode(0o600)).unwrap();
        configured_options(options, directory, clock)
    });
    let path = journal_path(&fixture);
    run(async {
        let host = fixture.host.take().unwrap();
        let completion = host.terminal_shutdown_completion().unwrap();
        let options = NativeInteractiveSessionOptions::new(
            fixture.workspace.clone(),
            host.loaded_config().config().model_preferences(),
        )
        .unwrap();
        let mut owner = NativeInteractiveSession::open_managed(
            host,
            directory(&path),
            options,
            NativeInteractiveInitialSession::Fresh,
            1,
        )
        .await
        .unwrap();
        assert!(owner.mcp_startup_failure().is_some());
        owner
            .enqueue("blocked until required readiness is repaired".into())
            .unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeInteractiveOutcome::Turn(Err(_))
        ));
        assert!(fixture.transport.requests.lock().unwrap().is_empty());
        fs::write(
            fixture.workspace.parent().unwrap().join("profile/mcp.json"),
            r#"{"mcp":{}}"#,
        )
        .unwrap();
        owner
            .request_control(
                NativeInteractiveControl::Mcp {
                    command: crate::mcp::commands::McpCommand::Reload,
                },
                11,
            )
            .unwrap();
        let reloaded = poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 12);
            owner
                .take_control_outcome()
                .map_or(Poll::Pending, Poll::Ready)
        })
        .await;
        assert!(reloaded.result.is_ok(), "{reloaded:?}");
        assert!(owner.mcp_startup_failure().is_none());
        owner.enqueue("run after explicit repair".into()).unwrap();
        assert!(matches!(
            outcome_at(&mut owner, 13).await,
            NativeInteractiveOutcome::Turn(Ok(_))
        ));
        owner.request_shutdown();
        assert!(matches!(
            outcome_at(&mut owner, 14).await,
            NativeInteractiveOutcome::Shutdown
        ));
        drop(owner);
        completion.wait().await;
        let workers = NativeOwnedWorkerScope::new();
        let journal =
            ManagedJournal::open(directory(&path), workers.clone(), JournalLimits::default())
                .await
                .unwrap();
        drop(journal);
        workers.close();
        workers.completion().wait().await;
    });
    assert_eq!(fixture.transport.requests.lock().unwrap().len(), 1);
}
