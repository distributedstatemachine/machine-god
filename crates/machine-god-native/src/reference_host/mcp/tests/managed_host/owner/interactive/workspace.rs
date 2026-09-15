use super::*;

fn scope(runtime: &NativeConversationRuntime) -> NativeWorkspaceScopeSnapshot {
    runtime.workspace_authority().unwrap().snapshot().unwrap()
}

async fn apply(
    owner: &mut NativeInteractiveSession,
    store: &Arc<NativeUserConfigStore>,
    action: NativeWorkspaceAction,
) -> NativeWorkspaceReceipt {
    owner
        .request_control(
            NativeInteractiveControl::Workspace {
                action,
                store: Some(store.clone()),
            },
            20,
        )
        .unwrap();
    let result = poll_fn(|cx| {
        let _ = owner.poll_progress(cx, 21);
        owner
            .take_control_outcome()
            .map_or(Poll::Pending, Poll::Ready)
    })
    .await;
    let NativeInteractiveControlReceipt::Workspace(receipt) = result.result.unwrap() else {
        panic!("workspace receipt");
    };
    receipt
}

#[test]
fn managed_workspace_edits_target_actual_parent_and_transition_forks_settled_selection() {
    let mut fixture = Fixture::with_options("auto", true, options);
    let path = journal_path(&fixture);
    let shared = fixture.workspace.parent().unwrap().join("shared");
    fs::create_dir(&shared).unwrap();
    let target = shared.join("actual.txt");
    let store = Arc::new(NativeUserConfigStore::new(
        fixture.workspace.parent().unwrap().join("settings"),
    ));
    run(async {
        let host = fixture.host.take().unwrap();
        let default_workspace = host.workspace_binding.as_ref().unwrap().authority.clone();
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
        let old = owner.runtime().clone();
        assert!(scope(&old).route(&target).is_err());
        let added = apply(
            &mut owner,
            &store,
            NativeWorkspaceAction::Add(shared.clone()),
        )
        .await;
        assert_eq!(added.runtime_changed, Some(true));
        assert!(scope(&old).route(&target).is_ok());
        assert!(
            default_workspace
                .snapshot()
                .unwrap()
                .route(&target)
                .is_err(),
            "foreground mutation must not replace host defaults"
        );
        owner
            .request_transition(NativeInteractiveTransition::New, 30)
            .unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeInteractiveOutcome::Transition(_)
        ));
        assert_ne!(old.id(), owner.runtime().id());
        assert!(scope(owner.runtime()).route(&target).is_ok());
        fixture.transport.responses.lock().unwrap().extend([
            call(
                "write",
                "write_file",
                &serde_json::json!({"path":target,"content":"actual parent scope"}),
            ),
            answer(),
        ]);
        owner
            .enqueue("write in the selected additional root".into())
            .unwrap();
        match outcome_at(&mut owner, 40).await {
            NativeInteractiveOutcome::Turn(Ok(_)) => {}
            NativeInteractiveOutcome::Turn(Err(error)) => panic!("parent turn: {error:?}"),
            _ => panic!("parent turn outcome"),
        }
        assert_eq!(fs::read(&target).unwrap(), b"actual parent scope");
        let cleared = apply(&mut owner, &store, NativeWorkspaceAction::Clear).await;
        assert!(cleared.snapshot.route(&target).is_err());
        assert!(
            scope(&old).route(&target).is_ok(),
            "replacement has an independent mutable selection"
        );
        assert!(
            default_workspace
                .snapshot()
                .unwrap()
                .route(&target)
                .is_err()
        );
        owner.request_shutdown();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeInteractiveOutcome::Shutdown
        ));
        drop(owner);
        drop(old);
        completion.wait().await;
    });
}
