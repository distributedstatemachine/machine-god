use super::*;
use crate::reference_host::NativeReferenceHostManagedOptions;
mod owner;

fn options(
    options: NativeReferenceHostConversationOptions,
    directory: &Directory,
    clock: Arc<Clock>,
) -> NativeReferenceHostConversationOptions {
    let workspace = directory.0.join("workspace");
    let environment =
        NativeEnvironment::new(None, Some(directory.0.join("state").into_os_string()), None);
    let selection = NativeRootSelection::from_environment(&environment, &workspace).unwrap();
    let open = |path: &std::path::Path| {
        rustix::fs::open(
            path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY,
            rustix::fs::Mode::empty(),
        )
        .unwrap()
    };
    let authority = NativeWorkspaceAuthority::open_blocking(
        open(&workspace),
        workspace,
        Some(open(selection.state_root())),
        selection.state_root().to_owned(),
        vec![],
        false,
    )
    .unwrap();
    options
        .with_workspace(authority, Arc::new(NativeWorkspaceContexts::new()))
        .with_model_routes(Arc::new(NativeConversationModelRoutes::new()))
        .with_observations(Arc::new(NativeConversationObservations::new()))
        .with_managed_agents(NativeReferenceHostManagedOptions::new(clock))
}

#[test]
fn managed_selection_requires_each_actual_shared_service_without_reading_clock() {
    let fixture = Fixture::with_options("ask", false, |options, directory, clock| {
        let selected = self::options(options, directory, clock.clone());
        let config = LoadedNativeConfig::from_file(NativeConfig::default());
        for missing in 0..6 {
            let mut candidate: PreparedCompositionOptions = selected.clone().into();
            match missing {
                0 => candidate.terminal = None,
                1 => candidate.permissions = None,
                2 => candidate.workspace_binding = None,
                3 => candidate.model_routes = None,
                4 => candidate.observations = None,
                5 => candidate.undo_tracker = None,
                _ => unreachable!(),
            }
            assert_eq!(
                validate_prepared_selections(&config, &candidate)
                    .unwrap_err()
                    .kind(),
                NativeReferenceHostBuildErrorKind::ManagedConfig,
            );
        }
        let candidate: PreparedCompositionOptions = selected.clone().into();
        validate_prepared_selections(&config, &candidate).unwrap();
        let selection = crate::reference_host::managed_host::select(&candidate)
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(
            &selection.budget,
            &selected.undo_tracker.shared_budget()
        ));
        assert_eq!(clock.0.load(Ordering::Relaxed), 0);
        selected
    });
    assert!(fixture.host().managed.is_some());
    assert!(fixture.host().services.managed_mcp_seed.is_some());
    assert!(
        fixture
            .host()
            .managed
            .as_ref()
            .unwrap()
            .parent_mcp
            .is_some()
    );
    assert!(fixture.host().mcp_runtime().is_none());
    assert!(fixture.host().mcp_controller().is_none());
    assert!(fixture.host().mcp_ephemeral_owner().is_none());
    assert!(fixture.host().mcp_contexts().is_none());
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
    assert_eq!(fixture.clock.0.load(Ordering::Relaxed), 0);
}

#[test]
fn managed_engine_routes_do_not_retain_outer_owners() {
    let mut fixture = Fixture::with_options("ask", true, options);
    let host = fixture.host.take().unwrap();
    let assembly = host.managed.as_ref().unwrap();
    let principals = Arc::downgrade(&assembly.principals);
    let mcp = Arc::downgrade(&assembly.mcp);
    let notices = Arc::downgrade(&assembly.notices);
    let completion = host.terminal_shutdown_completion().unwrap();
    let engine = host.into_engine();
    assert!(principals.upgrade().is_none());
    assert!(mcp.upgrade().is_none());
    assert!(notices.upgrade().is_none());
    assert_eq!(engine.tool_specs().len(), 26);
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
    drop(engine);
    completion.wait_on_worker().unwrap();
    assert!(completion.is_complete());
}

#[test]
fn managed_tools_reject_unregistered_parent_instead_of_using_host_global_authority() {
    let fixture = Fixture::with_options("ask", true, options);
    fixture.transport.responses.lock().unwrap().extend([
        call(
            "write",
            WRITE_FILE_TOOL_NAME,
            &serde_json::json!({"path":"forbidden.txt","content":"unregistered"}),
        ),
        answer(),
    ]);
    run(async {
        let conversation = fixture.conversation().await;
        let runtime = NativeConversationRuntime::new(
            conversation,
            fixture.host().loaded_config().config().model_preferences(),
            None,
        )
        .unwrap();
        let events = collect(&runtime).await;
        assert!(!events.iter().any(|event| matches!(event,
            TurnEvent::ToolFinished { call_id, output } if call_id.as_str() == "write" && !output.is_error
        )));
        assert!(!fixture.workspace.join("forbidden.txt").exists());
        assert_eq!(fixture.prompt.calls.load(Ordering::Relaxed), 0);
    });
}
