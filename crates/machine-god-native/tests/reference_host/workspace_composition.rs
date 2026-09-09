use super::*;
use machine_god_native::{
    NativeConversation, NativeConversationRuntime, NativePermissionContexts,
    NativeReferenceHostPermissionOptions, NativeSessionMetadata, NativeWorkspaceAuthority,
    NativeWorkspaceContexts, NativeWorkspaceEntrySpec, NativeWorkspaceSource,
    TokioPermissionReviewClock,
};
use permission_composition::{answer, call, configured, run_conversation};

fn authority(primary: &Path, state: &Path, additional: &Path) -> NativeWorkspaceAuthority {
    NativeWorkspaceAuthority::open_blocking(
        fs::File::open(primary).unwrap().into(),
        primary.canonicalize().unwrap(),
        Some(fs::File::open(state).unwrap().into()),
        state.canonicalize().unwrap(),
        vec![
            NativeWorkspaceEntrySpec::new(
                NativeWorkspaceSource::new(
                    additional.canonicalize().unwrap(),
                    additional.canonicalize().unwrap(),
                    true,
                )
                .unwrap(),
                true,
                false,
            )
            .unwrap(),
        ],
        false,
    )
    .unwrap()
}

fn compose(
    prepared: PreparedNativeRoots,
    config: LoadedNativeConfig,
    options: NativeReferenceHostConversationOptions,
    responses: Vec<Vec<u8>>,
) -> NativeReferenceHost {
    NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots_and_conversation(
        config,
        Arc::new(ScriptedTransport::new("workspace-composed", responses)),
        production_gateway_target(),
        prepared,
        Arc::new(AllowingPrompter::default()),
        inert_question_prompter(),
        never_deadline(),
        options,
    )
    .unwrap()
}

fn collect(host: &NativeReferenceHost, bind_workspace: bool) -> Vec<TurnEvent> {
    collect_with_history(host, bind_workspace).0
}

fn collect_with_history(
    host: &NativeReferenceHost,
    bind_workspace: bool,
) -> (
    Vec<TurnEvent>,
    machine_god_native::NativeConversationHistory,
) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let conversation = NativeConversation::create(
                host.session_lifecycle(),
                NativeSessionMetadata::default(),
            )
            .await
            .unwrap();
            let conversation = match host.observations() {
                Some(observations) => conversation.with_observations(&observations).unwrap(),
                None => conversation,
            };
            let conversation = host
                .configure_conversation_permissions(conversation)
                .unwrap();
            let conversation = if bind_workspace {
                host.configure_conversation_workspace(conversation).unwrap()
            } else {
                conversation
            };
            let runtime = NativeConversationRuntime::new(
                conversation,
                host.loaded_config().config().model_preferences(),
                None,
            )
            .unwrap();
            let events = run_conversation(&runtime, 101).await;
            (events, runtime.history().unwrap())
        })
}

#[test]
fn workspace_host_executes_all_five_mutations_and_logical_undo_with_and_without_native_policy() {
    for governed in [false, true] {
        let temporary = TemporaryDirectory::new("workspace-five");
        let (prepared, state) = complete_terminal_roots(temporary.path());
        let primary = prepared.workspace_root().to_owned();
        let additional = temporary.path().join("additional");
        fs::create_dir(&additional).unwrap();
        let additional = additional.canonicalize().unwrap();
        seed_undo_files(&primary, "");
        seed_undo_files(&additional, "");
        let tracker = Arc::new(FileUndoTracker::new());
        let mut options = NativeReferenceHostConversationOptions::new(tracker.clone())
            .with_observations(Arc::new(
                machine_god_native::NativeConversationObservations::new(),
            ))
            .with_workspace(
                authority(&primary, &state, &additional),
                Arc::new(NativeWorkspaceContexts::new()),
            );
        let config = if governed {
            options = options
                .with_terminal(complete_terminal_options())
                .with_permissions(NativeReferenceHostPermissionOptions::new(
                    Arc::new(NativePermissionContexts::new()),
                    Arc::new(TokioPermissionReviewClock),
                ));
            configured(temporary.path(), "yolo", &json!([]))
        } else {
            built_in_config()
        };
        let mut responses = Vec::new();
        for (index, (name, mut input)) in undo_mutations().into_iter().enumerate() {
            let key = match name {
                "rename_file" => "old_path",
                "copy_file" => "source",
                _ => "path",
            };
            input[key] = additional
                .join(input[key].as_str().unwrap())
                .to_str()
                .unwrap()
                .into();
            responses.push(format!(
                "data: {}\n\ndata: {{\"type\":\"finish\",\"finishReason\":{{\"unified\":\"tool-calls\"}}}}\n\n",
                json!({"type":"tool-call", "toolCallId":format!("workspace-{index}"), "toolName":name, "input":input}),
            ).into_bytes());
        }
        responses.push(answer());
        let host = compose(prepared, config, options, responses);
        let completion = host.terminal_shutdown_completion();
        let (events, history) = collect_with_history(&host, true);
        assert_mutation_history(&history, &additional);
        let outputs: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                TurnEvent::ToolFinished { output, .. } => Some(output),
                _ => None,
            })
            .collect();
        assert_eq!(outputs.len(), 5);
        assert!(outputs.iter().all(|output| !output.is_error), "{outputs:?}");
        assert_eq!(
            fs::read_to_string(additional.join("w.txt")).unwrap(),
            "new write"
        );
        assert_eq!(
            fs::read_to_string(additional.join("e.txt")).unwrap(),
            "after edit"
        );
        assert!(!additional.join("d.txt").exists() && !additional.join("r.txt").exists());
        assert_eq!(
            fs::read_to_string(primary.join("copy.txt")).unwrap(),
            "copied"
        );
        assert_eq!(
            fs::read_to_string(primary.join("renamed.txt")).unwrap(),
            "renamed"
        );
        assert_eq!(
            fs::read_to_string(primary.join("w.txt")).unwrap(),
            "old write"
        );
        drop(host);
        if let Some(completion) = completion {
            completion.wait_on_worker().unwrap();
        }
        let cancel = CancellationToken::new();
        assert_eq!(
            tracker.undo_last(&cancel).unwrap(),
            FileUndoOutcome::Removed("copy.txt".into())
        );
        for name in ["r.txt", "d.txt", "e.txt", "w.txt"] {
            assert_eq!(
                tracker.undo_last(&cancel).unwrap(),
                FileUndoOutcome::Restored(additional.join(name).to_str().unwrap().into())
            );
        }
        assert_undo_files_original(&primary, "");
        assert_undo_files_original(&additional, "");
    }
}

fn assert_mutation_history(
    history: &machine_god_native::NativeConversationHistory,
    additional: &Path,
) {
    let evidence: Vec<_> = history
        .groups()
        .iter()
        .flat_map(machine_god_native::NativeHistoryGroup::files)
        .collect();
    assert_eq!(evidence.len(), 5, "{history:?}");
    for (file, name) in evidence
        .iter()
        .zip(["w.txt", "e.txt", "d.txt", "r.txt", "c.txt"])
    {
        assert_eq!(file.path(), additional.join(name).to_str().unwrap());
        assert_eq!(
            file.status(),
            machine_god_native::NativeHistoryFileStatus::Success
        );
    }
    assert_eq!(evidence[3].new_path(), Some("renamed.txt"));
    assert_eq!(evidence[4].new_path(), Some("copy.txt"));
}

#[test]
fn workspace_host_scoped_read_requires_conversation_binding() {
    for bind in [false, true] {
        let temporary = TemporaryDirectory::new("workspace-read-binding");
        let (prepared, state) = complete_terminal_roots(temporary.path());
        let primary = prepared.workspace_root().to_owned();
        let additional = temporary.path().join("additional");
        fs::create_dir(&additional).unwrap();
        fs::write(additional.join("selected"), "additional contents").unwrap();
        let selected = additional.canonicalize().unwrap().join("selected");
        let options = NativeReferenceHostConversationOptions::new(Arc::new(FileUndoTracker::new()))
            .with_observations(Arc::new(
                machine_god_native::NativeConversationObservations::new(),
            ))
            .with_workspace(
                authority(&primary, &state, &additional),
                Arc::new(NativeWorkspaceContexts::new()),
            );
        let host = compose(
            prepared,
            built_in_config(),
            options,
            vec![call("read_file", &json!({"path": selected})), answer()],
        );
        let events = collect(&host, bind);
        assert_eq!(
            events.iter().any(
                |event| matches!(event, TurnEvent::ToolFinished { output, .. } if !output.is_error)
            ),
            bind
        );
    }
}

#[test]
fn workspace_host_routes_metadata_enumeration_and_grep_with_policy_and_history() {
    for name in [
        "file_info",
        "list_files",
        "glob_files",
        "grep_files",
        "create_folder",
    ] {
        let temporary = TemporaryDirectory::new("workspace-tool-catalog");
        let (prepared, state) = complete_terminal_roots(temporary.path());
        let primary = prepared.workspace_root().to_owned();
        let additional = temporary.path().join("additional");
        fs::create_dir(&additional).unwrap();
        let additional = additional.canonicalize().unwrap();
        fs::write(additional.join("selected.txt"), "needle from additional").unwrap();
        fs::write(primary.join("primary.txt"), "not selected").unwrap();
        let input = match name {
            "file_info" => json!({"path": additional.join("selected.txt")}),
            "list_files" => json!({"path": additional}),
            "glob_files" => json!({"path": additional, "pattern":"*.txt"}),
            "grep_files" => {
                json!({"path": additional, "pattern":"needle", "mode":"files_with_matches"})
            }
            "create_folder" => json!({"path": additional.join("created")}),
            _ => unreachable!(),
        };
        let host = compose(
            prepared,
            configured(temporary.path(), "yolo", &json!([])),
            NativeReferenceHostConversationOptions::new(Arc::new(FileUndoTracker::new()))
                .with_workspace(
                    authority(&primary, &state, &additional),
                    Arc::new(NativeWorkspaceContexts::new()),
                )
                .with_terminal(complete_terminal_options())
                .with_permissions(NativeReferenceHostPermissionOptions::new(
                    Arc::new(NativePermissionContexts::new()),
                    Arc::new(TokioPermissionReviewClock),
                ))
                .with_observations(Arc::new(
                    machine_god_native::NativeConversationObservations::new(),
                )),
            vec![call(name, &input), answer()],
        );
        let completion = host.terminal_shutdown_completion().unwrap();
        let (events, history) = collect_with_history(&host, true);
        let output = events
            .iter()
            .find_map(|event| match event {
                TurnEvent::ToolFinished { output, .. } => Some(output),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{name}: no output"));
        assert!(!output.is_error, "{name}: {output:?}");
        if name == "create_folder" {
            assert!(additional.join("created").is_dir());
            assert!(!primary.join("created").exists());
        } else {
            assert!(
                output.content.to_string().contains("selected.txt"),
                "{name}: {output:?}"
            );
            assert!(!output.content.to_string().contains("primary.txt"));
        }
        let evidence: Vec<_> = history
            .groups()
            .iter()
            .flat_map(machine_god_native::NativeHistoryGroup::files)
            .collect();
        if matches!(name, "list_files" | "glob_files" | "grep_files") {
            assert_eq!(evidence.len(), 1, "{name}");
            assert_eq!(evidence[0].path(), additional.to_str().unwrap());
            assert_eq!(
                evidence[0].status(),
                machine_god_native::NativeHistoryFileStatus::Success
            );
        } else {
            assert!(evidence.is_empty());
        }
        drop(host);
        completion.wait_on_worker().unwrap();
    }
}

#[test]
fn workspace_host_rejects_foreign_primary_or_state_before_transport() {
    for wrong_primary in [false, true] {
        let temporary = TemporaryDirectory::new("workspace-binding-mismatch");
        let (prepared, state) = complete_terminal_roots(temporary.path());
        let primary = prepared.workspace_root().to_owned();
        let extra = temporary.path().join("extra");
        let foreign = temporary.path().join("foreign");
        fs::create_dir(&extra).unwrap();
        fs::create_dir(&foreign).unwrap();
        let selected = if wrong_primary {
            authority(&foreign, &state, &extra)
        } else {
            authority(&primary, &foreign, &extra)
        };
        let transport = ScriptedTransport::new("mismatched-roots", Vec::<Vec<u8>>::new());
        let result = NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots_and_conversation(
            built_in_config(), Arc::new(transport.clone()), production_gateway_target(), prepared,
            Arc::new(AllowingPrompter::default()), inert_question_prompter(), never_deadline(),
            NativeReferenceHostConversationOptions::new(Arc::new(FileUndoTracker::new())).with_workspace(selected, Arc::new(NativeWorkspaceContexts::new())),
        );
        assert_eq!(
            result.err().unwrap().kind(),
            NativeReferenceHostBuildErrorKind::WorkspaceRoot
        );
        assert!(transport.requests().is_empty());
    }
}
