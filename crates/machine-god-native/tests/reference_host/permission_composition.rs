use super::*;
use machine_god_native::{
    NativeConversation, NativeConversationRuntime, NativePermissionContexts,
    NativeReferenceHostPermissionOptions, NativeSessionMetadata, NativeSessionOrigin,
    TokioPermissionReviewClock,
};

pub(super) fn configured(base: &Path, mode: &str, rules: &Value) -> LoadedNativeConfig {
    configured_sandbox(base, mode, rules, "none")
}

pub(super) fn configured_sandbox(
    base: &Path,
    mode: &str,
    rules: &Value,
    sandbox: &str,
) -> LoadedNativeConfig {
    load_config(
        base,
        &json!({
            "schema_version":5, "permission_mode":mode, "sandbox_mode":sandbox,
            "permission_rules":rules, "provider":"vercel_ai_gateway",
            "transport":"ai_gateway_http", "credential_source":"environment",
            "model":"private/main", "effort":"auto", "fast_mode":false,
        })
        .to_string(),
    )
}

fn options() -> NativeReferenceHostConversationOptions {
    NativeReferenceHostConversationOptions::new(Arc::new(FileUndoTracker::new()))
        .with_terminal(complete_terminal_options())
        .with_permissions(NativeReferenceHostPermissionOptions::new(
            Arc::new(NativePermissionContexts::new()),
            Arc::new(TokioPermissionReviewClock),
        ))
}

pub(super) fn call(name: &str, input: &Value) -> Vec<u8> {
    format!("data: {}\n\ndata: {{\"type\":\"finish\",\"finishReason\":{{\"unified\":\"tool-calls\"}}}}\n\n",
        json!({"type":"tool-call","toolCallId":"actual-call","toolName":name,"input":input}))
        .into_bytes()
}

pub(super) fn answer() -> Vec<u8> {
    b"data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"complete\"}\n\ndata: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n".to_vec()
}

#[cfg(target_os = "macos")]
#[test]
fn permission_host_actual_terminal_enforces_os_and_preserves_explicit_none_and_yolo() {
    use machine_god_native::NATIVE_SANDBOX_EXECUTABLE;
    use std::fs::File;
    let helper = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/machine-god"),
        PathBuf::from,
    );
    assert!(
        helper.is_file(),
        "build the release CLI before native process tests"
    );
    for (mode, sandbox, executable, outside_allowed) in [
        ("ask", "os", true, false),
        ("ask", "os", false, false),
        ("yolo", "os", false, true),
        ("ask", "none", false, true),
    ] {
        let temporary = TemporaryDirectory::new("permission-terminal-sandbox");
        let (prepared, _) = complete_terminal_roots(temporary.path());
        let workspace = temporary.path().join("workspace");
        let outside = temporary.path().canonicalize().unwrap().join("outside");
        assert!(!outside.starts_with("/private/tmp") && !outside.starts_with("/tmp"));
        let quoted = outside.to_str().unwrap().replace('\'', "'\\''");
        let command = format!(
            "printf inside > inside; (printf outside > '{quoted}') 2>/dev/null; printf done"
        );
        let transport = ScriptedTransport::new(
            "permission-terminal",
            [
                call(
                    "terminal",
                    &json!({"action":"exec", "profile":"clean", "command":command}),
                ),
                answer(),
            ],
        );
        let mut permission = NativeReferenceHostPermissionOptions::new(
            Arc::new(NativePermissionContexts::new()),
            Arc::new(TokioPermissionReviewClock),
        );
        if executable {
            permission =
                permission.with_sandbox_executable(File::open(NATIVE_SANDBOX_EXECUTABLE).unwrap());
        }
        let options = NativeReferenceHostConversationOptions::new(Arc::new(FileUndoTracker::new()))
            .with_terminal(
                NativeReferenceHostTerminalOptions::new(
                    helper.clone(),
                    Some("/bin/bash".into()),
                    vec![],
                )
                .unwrap(),
            )
            .with_permissions(permission);
        let prompt = AllowingPrompter::default();
        let host = NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots_and_conversation(
            configured_sandbox(temporary.path(), mode, &json!([]), sandbox), Arc::new(transport.clone()),
            production_gateway_target(), prepared, Arc::new(prompt.clone()),
            inert_question_prompter(), never_deadline(), options,
        ).unwrap();
        let completion = host.terminal_shutdown_completion().unwrap();
        let events = collect(&host, &workspace);
        let missing_authority = mode == "ask" && sandbox == "os" && !executable;
        assert_eq!(workspace.join("inside").exists(), !missing_authority);
        assert_eq!(outside.exists(), outside_allowed);
        assert!(events.iter().any(|event| matches!(event,
            TurnEvent::ToolFinished { output, .. } if output.is_error == missing_authority
        )));
        assert_eq!(prompt.requests().len(), usize::from(mode == "ask"));
        assert_eq!(transport.requests().len(), 2);
        drop(host);
        completion.wait_on_worker().unwrap();
    }
}

fn collect(host: &NativeReferenceHost, workspace: &Path) -> Vec<TurnEvent> {
    collect_with_policy(host, workspace, None)
}

fn collect_with_policy(
    host: &NativeReferenceHost,
    workspace: &Path,
    policy: Option<machine_god_native::NativePermissionPolicySnapshot>,
) -> Vec<TurnEvent> {
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    executor.block_on(async {
        let conversation = NativeConversation::create(
            host.session_lifecycle(),
            NativeSessionMetadata::new(workspace, 100, NativeSessionOrigin::Cli).unwrap(),
        )
        .await
        .unwrap();
        let conversation = match policy {
            Some(policy) => {
                host.configure_conversation_permissions_with_policy(conversation, policy)
            }
            None => host.configure_conversation_permissions(conversation),
        }
        .unwrap();
        let runtime = NativeConversationRuntime::new(
            conversation,
            host.loaded_config().config().model_preferences(),
            None,
        )
        .unwrap();
        run_conversation(&runtime, 101).await
    })
}

pub(super) async fn run_conversation(
    runtime: &NativeConversationRuntime,
    now_ms: i64,
) -> Vec<TurnEvent> {
    runtime
        .enqueue("perform the requested workspace operation".into())
        .unwrap();
    let mut turn = runtime.start_next(now_ms).await.unwrap().unwrap();
    let mut events = Vec::new();
    while let Some(event) = turn.next().await {
        events.push(event.unwrap().payload);
    }
    assert_completed(&events);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, TurnEvent::Failed { .. }))
    );
    events
}

#[cfg(target_os = "macos")]
#[test]
fn terminal_start_and_monitor_continue_model_rounds_without_inventing_legacy_history() {
    use machine_god_native::NativeConversationObservations;
    let temporary = TemporaryDirectory::new("terminal-ordinary-history");
    let (prepared, _) = complete_terminal_roots(temporary.path());
    let workspace = temporary.path().join("workspace");
    let helper = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/machine-god"),
        PathBuf::from,
    );
    assert!(
        helper.is_file(),
        "build the release CLI before native process tests"
    );
    let observations = Arc::new(NativeConversationObservations::new());
    let options = options()
        .with_observations(observations.clone())
        .with_terminal(
            NativeReferenceHostTerminalOptions::new(
                helper,
                Some("/bin/bash".into()),
                vec![("PATH".into(), "/usr/bin:/bin".into())],
            )
            .unwrap(),
        );
    let transport = ScriptedTransport::new("terminal-ordinary-history", Vec::<Vec<u8>>::new());
    let host =
        NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots_and_conversation(
            configured(temporary.path(), "ask", &json!([])),
            Arc::new(transport.clone()),
            production_gateway_target(),
            prepared,
            Arc::new(AllowingPrompter::default()),
            inert_question_prompter(),
            never_deadline(),
            options,
        )
        .unwrap();
    let completion = host.terminal_shutdown_completion().unwrap();
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    executor.block_on(async {
        let conversation = NativeConversation::create(host.session_lifecycle(),
            NativeSessionMetadata::new(&workspace, 100, NativeSessionOrigin::Cli).unwrap(),
        ).await.unwrap().with_observations(&observations).unwrap();
        let conversation = host.configure_conversation_permissions(conversation).unwrap();
        let runtime = NativeConversationRuntime::new(conversation,
            host.loaded_config().config().model_preferences(), None,
        ).unwrap();
        let mut terminal_id = Value::Null;
        for round in 0..3_u32 {
            let input = match round {
                0 => json!({"action":"start", "profile":"clean", "command":"exec /bin/sleep 30"}),
                1 => json!({"action":"monitor", "session_id":terminal_id, "monitor":{"kind":"add", "definition":{
                    "condition":{"kind":"custom_probe", "command":"printf observed", "cwd":"."},
                    "check_interval_ms":100, "notify":{"kind":"on_match"}, "lifetime":{"kind":"until_session_end"}
                }}}),
                _ => json!({"action":"close", "session_id":terminal_id, "close_policy":"force"}),
            };
            transport.state.lock().unwrap().responses.extend([call("terminal", &input), answer()]);
            let events = run_conversation(&runtime, 101 + i64::from(round)).await;
            let output = events.iter().find_map(|event| match event {
                TurnEvent::ToolFinished { output, .. } => Some(output),
                _ => None,
            }).expect("ordinary completed tool output");
            assert!(!output.is_error, "round {round}: {:?}", output.content);
            if round == 0 {
                terminal_id = output.content["session"]["session_id"].clone();
                assert!(terminal_id.is_string());
            }
            assert_eq!(transport.requests().len(), usize::try_from((round + 1) * 2).unwrap());
            assert!(events.iter().any(|event| matches!(event, TurnEvent::Model { event: machine_god_core::ModelEvent::TextDelta { text } } if text == "complete")));
            let history = runtime.history().unwrap();
            assert_eq!(history.groups().len(), usize::try_from(round + 1).unwrap());
            assert!(history.groups().iter().all(|group| group.background().is_none()));
        }
    });
    drop(host);
    completion.wait_on_worker().unwrap();
}

#[test]
fn permission_host_wires_file_approval_and_ask_auto_yolo_without_extra_model_calls() {
    for (mode, rules, prompts) in [
        ("ask", json!([]), 1),
        ("auto", json!([]), 0),
        (
            "yolo",
            json!([{"permission":"write_file","pattern":"*","action":"deny"}]),
            0,
        ),
    ] {
        let temporary = TemporaryDirectory::new("permission-mode-write");
        let (prepared, _) = complete_terminal_roots(temporary.path());
        let workspace = temporary.path().join("workspace");
        let transport = ScriptedTransport::new(
            "permission-mode",
            [
                call(
                    "write_file",
                    &json!({"path":"result","content":"exact bytes"}),
                ),
                answer(),
            ],
        );
        let prompt = AllowingPrompter::default();
        let host = NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots_and_conversation(
            configured(temporary.path(), mode, &rules), Arc::new(transport.clone()),
            production_gateway_target(), prepared, Arc::new(prompt.clone()),
            inert_question_prompter(), never_deadline(), options(),
        ).unwrap();
        assert!(transport.requests().is_empty());
        assert!(prompt.requests().is_empty());
        assert!(!workspace.join("result").exists());
        let completion = host.terminal_shutdown_completion().unwrap();
        let events = collect(&host, &workspace);
        assert!(events.iter().any(
            |event| matches!(event, TurnEvent::ToolFinished { output, .. } if !output.is_error)
        ));
        assert_eq!(fs::read(workspace.join("result")).unwrap(), b"exact bytes");
        assert_eq!(prompt.requests().len(), prompts);
        assert_eq!(transport.requests().len(), 2);
        drop(host);
        completion.wait_on_worker().unwrap();
    }
}

#[test]
fn explicit_current_policy_overrides_config_defaults_through_actual_tools() {
    use machine_god_native::{
        NativeConfiguredPermissionRules, NativePermissionPolicySnapshot, PermissionMode,
    };
    for (mode, prompts) in [(PermissionMode::Ask, 1), (PermissionMode::Yolo, 0)] {
        let temporary = TemporaryDirectory::new("explicit-current-policy");
        let (prepared, _) = complete_terminal_roots(temporary.path());
        let workspace = temporary.path().join("workspace");
        let transport = ScriptedTransport::new(
            "explicit-current-policy",
            [
                call(
                    "write_file",
                    &json!({"path":"result", "content":"selected policy"}),
                ),
                answer(),
            ],
        );
        let prompt = AllowingPrompter::default();
        let host = NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots_and_conversation(
            configured(temporary.path(), "ask", &json!([{"permission":"write_file", "pattern":"*", "action":"deny"}])),
            Arc::new(transport.clone()), production_gateway_target(), prepared,
            Arc::new(prompt.clone()), inert_question_prompter(), never_deadline(), options(),
        ).unwrap();
        let completion = host.terminal_shutdown_completion().unwrap();
        let policy = NativePermissionPolicySnapshot::new(
            mode,
            Arc::new(NativeConfiguredPermissionRules::default()),
        );
        let events = collect_with_policy(&host, &workspace, Some(policy));
        assert!(events.iter().any(|event| matches!(event,
            TurnEvent::ToolFinished { output, .. } if !output.is_error
        )));
        assert_eq!(
            fs::read(workspace.join("result")).unwrap(),
            b"selected policy"
        );
        assert_eq!(prompt.requests().len(), prompts);
        assert_eq!(transport.requests().len(), 2);
        drop(host);
        completion.wait_on_worker().unwrap();
    }
}

#[test]
fn permission_host_canonical_grep_executes_and_configured_deny_prevents_read() {
    for denied in [false, true] {
        let temporary = TemporaryDirectory::new("permission-grep");
        let (prepared, _) = complete_terminal_roots(temporary.path());
        let workspace = temporary.path().join("workspace");
        fs::write(workspace.join("source"), "needle\n").unwrap();
        let rules = if denied {
            json!([{"permission":"grep_files","pattern":"*","action":"deny"}])
        } else {
            json!([])
        };
        let transport = ScriptedTransport::new(
            "permission-grep",
            [call("grep_files", &json!({"pattern":"needle"})), answer()],
        );
        let prompt = AllowingPrompter::default();
        let host = NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots_and_conversation(
            configured(temporary.path(), "ask", &rules), Arc::new(transport.clone()),
            production_gateway_target(), prepared, Arc::new(prompt.clone()),
            inert_question_prompter(), never_deadline(), options(),
        ).unwrap();
        let completion = host.terminal_shutdown_completion().unwrap();
        let events = collect(&host, &workspace);
        if denied {
            assert!(events.iter().any(|event| matches!(
                event,
                TurnEvent::PermissionResolved {
                    decision: machine_god_core::PermissionDecision::Deny { .. },
                    ..
                }
            )));
            assert!(!events.iter().any(|event| matches!(
                event,
                TurnEvent::ToolStarted { .. } | TurnEvent::ToolFinished { .. }
            )));
            assert!(
                body(&transport.requests()[1])
                    .to_string()
                    .contains("permission_denied")
            );
        } else {
            assert!(events.iter().any(
                |event| matches!(event, TurnEvent::ToolFinished { output, .. } if !output.is_error)
            ));
        }
        assert!(prompt.requests().is_empty());
        assert_eq!(transport.requests().len(), 2);
        drop(host);
        completion.wait_on_worker().unwrap();
    }
}

#[test]
fn permission_host_auto_delete_uses_actual_dedicated_reviewer_and_replans_on_ask() {
    for allowed in [false, true] {
        let temporary = TemporaryDirectory::new("permission-auto-review");
        let (prepared, _) = complete_terminal_roots(temporary.path());
        let workspace = temporary.path().join("workspace");
        fs::write(workspace.join("selected"), "before\n").unwrap();
        let assessment = call(
            "permission_decision",
            &json!({
                "risk":"low", "authorization":"high", "decision":if allowed {"allow"} else {"ask"},
                "rationale":"bounded explicit operation",
            }),
        );
        let transport = ScriptedTransport::new(
            "permission-auto-review",
            [
                call("delete_file", &json!({"path":"selected"})),
                assessment,
                answer(),
            ],
        );
        let prompt = AllowingPrompter::default();
        let host = NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots_and_conversation(
            configured(temporary.path(), "auto", &json!([])), Arc::new(transport.clone()),
            production_gateway_target(), prepared, Arc::new(prompt.clone()),
            inert_question_prompter(), never_deadline(), options(),
        ).unwrap();
        let completion = host.terminal_shutdown_completion().unwrap();
        collect(&host, &workspace);
        assert_eq!(workspace.join("selected").exists(), !allowed);
        assert!(prompt.requests().is_empty());
        let requests = transport.requests();
        assert_eq!(requests.len(), 3);
        let model = |request: &CapturedRequest| {
            request
                .headers
                .iter()
                .find(|(name, _)| name == "ai-language-model-id")
                .map(|(_, value)| value.clone())
                .unwrap()
        };
        assert_eq!(model(&requests[0]), "private/main");
        let review = body(&requests[1]);
        assert_eq!(
            model(&requests[1]),
            machine_god_native::NATIVE_PERMISSION_REVIEW_MODEL
        );
        assert!(
            review
                .to_string()
                .contains("perform the requested workspace operation")
        );
        assert_eq!(model(&requests[2]), "private/main");
        drop(host);
        completion.wait_on_worker().unwrap();
    }
}

#[test]
fn permission_options_require_complete_terminal_before_preparing_namespaces() {
    let temporary = TemporaryDirectory::new("permission-missing-terminal");
    let (prepared, state) = complete_terminal_roots(temporary.path());
    let options = NativeReferenceHostConversationOptions::new(Arc::new(FileUndoTracker::new()))
        .with_permissions(NativeReferenceHostPermissionOptions::new(
            Arc::new(NativePermissionContexts::new()),
            Arc::new(TokioPermissionReviewClock),
        ));
    let transport = ScriptedTransport::new("permission-inert", Vec::<Vec<u8>>::new());
    let error =
        NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots_and_conversation(
            built_in_config(),
            Arc::new(transport.clone()),
            production_gateway_target(),
            prepared,
            Arc::new(AllowingPrompter::default()),
            inert_question_prompter(),
            never_deadline(),
            options,
        )
        .unwrap_err();
    assert_eq!(
        error.kind(),
        NativeReferenceHostBuildErrorKind::PermissionConfig
    );
    assert!(!state.join("terminal-startup").exists());
    assert!(transport.requests().is_empty());
}
