use super::*;
use crate::*;
use futures_executor::block_on;
use futures_util::StreamExt;
use serde_json::json;
use std::sync::atomic::Ordering;

mod fixture;
use fixture::*;

fn standard(mode: PermissionMode, decision: &'static str) -> Fixture {
    Fixture::new(
        mode,
        decision,
        PermissionPromptDecision::AllowOnce,
        r#"{"type":"object"}"#,
        vec![json!({"event":1})],
        NativeConfiguredPermissionRules::default(),
    )
}

#[test]
fn controller_modes_and_auto_uncertainty_keep_native_semantics() {
    for (mode, decision, expected_writes, prompts, reviews) in [
        (PermissionMode::Ask, "allow", 1, 1, 0),
        (PermissionMode::Auto, "allow", 1, 0, 1),
        (PermissionMode::Auto, "ask", 0, 0, 1),
        (PermissionMode::Yolo, "ask", 1, 0, 0),
    ] {
        let fixture = standard(mode, decision);
        fixture.run();
        assert_eq!(fixture.write_count(), expected_writes);
        assert_eq!(fixture.prompt.calls.load(Ordering::SeqCst), prompts);
        assert_eq!(fixture.transport.wire.lock().unwrap().len(), reviews);
        assert_eq!(fixture.builtin.calls.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.preparer.active.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn configured_rules_override_review_except_explicit_yolo() {
    for (mode, decision, writes, prompts) in [
        (
            PermissionMode::Auto,
            NativeConfiguredPermissionDecision::Allow,
            1,
            0,
        ),
        (
            PermissionMode::Auto,
            NativeConfiguredPermissionDecision::Deny,
            0,
            0,
        ),
        (
            PermissionMode::Auto,
            NativeConfiguredPermissionDecision::Ask,
            1,
            1,
        ),
        (
            PermissionMode::Yolo,
            NativeConfiguredPermissionDecision::Deny,
            1,
            0,
        ),
    ] {
        let rule = NativeConfiguredPermissionRule::new(NAME, "*", decision).unwrap();
        let fixture = Fixture::new(
            mode,
            "ask",
            PermissionPromptDecision::AllowOnce,
            r#"{"type":"object"}"#,
            vec![json!({})],
            NativeConfiguredPermissionRules::new(vec![rule]).unwrap(),
        );
        fixture.run();
        assert_eq!(fixture.write_count(), writes);
        assert_eq!(fixture.prompt.calls.load(Ordering::SeqCst), prompts);
        assert!(fixture.transport.wire.lock().unwrap().is_empty());
    }
}

#[test]
fn exact_grants_reuse_only_matching_arguments_and_overflow_does_not_block_execution() {
    for decision in [
        PermissionPromptDecision::AllowTurn,
        PermissionPromptDecision::AllowSession,
    ] {
        for size in [1, 4100] {
            let value = json!({"text":"x".repeat(size)});
            let fixture = Fixture::new(
                PermissionMode::Ask,
                "ask",
                decision,
                r#"{"type":"object"}"#,
                vec![value.clone(), value, json!({"changed":true})],
                NativeConfiguredPermissionRules::default(),
            );
            fixture.run();
            assert_eq!(fixture.write_count(), 3);
            assert_eq!(
                fixture.prompt.calls.load(Ordering::SeqCst),
                if size == 1 { 2 } else { 3 }
            );
            assert_eq!(
                fixture.prompt.reusable.load(Ordering::SeqCst),
                if size == 1 { 2 } else { 1 }
            );
        }
    }
}

#[test]
fn saved_rules_bind_exact_action_and_yolo_retains_its_existing_bypass() {
    for (mode, decision, writes) in [
        (PermissionMode::Ask, NativePermissionRuleDecision::Allow, 1),
        (PermissionMode::Ask, NativePermissionRuleDecision::Deny, 0),
        (PermissionMode::Yolo, NativePermissionRuleDecision::Deny, 1),
    ] {
        let fixture = standard(mode, "ask");
        let owner = fixture.runtime.permissions().unwrap();
        let key = fixture.key(&json!({"event":1}));
        assert!(!key.canonical().contains("secret-credential"));
        assert!(!key.canonical().contains("secret-config"));
        let proposal = owner
            .propose_rule_change(NativePermissionRuleChange::Set {
                key,
                display_identity: "Exact calendar action".into(),
                decision,
            })
            .unwrap();
        block_on(owner.confirm_rule_change(proposal)).unwrap();
        fixture.run();
        assert_eq!(fixture.write_count(), writes);
        assert_eq!(fixture.prompt.calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn auto_reviews_required_server_authoritative_schema_and_exact_json_through_final_write() {
    let schema = r#"{"type":"object","pattern":"(?=a)","properties":{"tiny":{"type":"number","minimum":1e-400}}}"#;
    let arguments = machine_god_core::json::from_str(r#"{"tiny":1e-400,"huge":1e400,"precise":9007199254740993.00001,"zero":-0,"$serde_json::private::Number":"1e900","nested":{"$serde_json::private::RawValue":"1e800"}}"#).unwrap();
    let fixture = Fixture::new(
        PermissionMode::Auto,
        "allow",
        PermissionPromptDecision::Deny,
        schema,
        vec![arguments.clone()],
        NativeConfiguredPermissionRules::default(),
    );
    fixture.run();
    assert_eq!(fixture.write_count(), 1);
    let wire = fixture.transport.wire.lock().unwrap();
    assert_eq!(wire[0]["prompt"][1]["content"][0]["input"], arguments);
    assert!(
        serde_json::to_string(&wire[0])
            .unwrap()
            .contains("schema_json")
    );
    let body = fixture.writes.lock().unwrap();
    let transmitted = machine_god_core::json::from_slice(&body).unwrap();
    assert_eq!(transmitted["params"]["arguments"], arguments);
    assert_eq!(
        serde_json::to_string(&transmitted["params"]["arguments"]).unwrap(),
        serde_json::to_string(&arguments).unwrap()
    );
}

#[test]
fn unknown_failed_and_retired_runtime_routes_never_fall_back_to_builtin() {
    for fault in 1..=6 {
        let fixture = standard(PermissionMode::Yolo, "allow");
        fixture.authority.fault.store(fault, Ordering::SeqCst);
        let events = block_on(fixture.start().collect::<Vec<_>>());
        assert!(events.iter().all(Result::is_ok));
        assert_eq!(fixture.write_count(), 0);
        assert_eq!(fixture.builtin.calls.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.prompt.calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn revocation_during_review_before_claim_and_after_claim_prevents_any_write() {
    for point in 1..=3 {
        let fixture = standard(PermissionMode::Auto, "allow");
        fixture.hooks.revoke.store(point, Ordering::SeqCst);
        let events = block_on(fixture.start().collect::<Vec<_>>());
        assert!(events.iter().all(Result::is_ok));
        assert_eq!(fixture.write_count(), 0);
    }
}

#[test]
fn dropping_pending_review_releases_owned_reservation_and_prepare_permit() {
    let fixture = standard(PermissionMode::Auto, "pending");
    let mut turn = fixture.start();
    block_on(std::future::poll_fn(|cx| {
        loop {
            use futures_core::Stream;
            match std::pin::Pin::new(&mut turn).poll_next(cx) {
                std::task::Poll::Pending => {
                    return if fixture.transport.wire.lock().unwrap().is_empty() {
                        std::task::Poll::Pending
                    } else {
                        std::task::Poll::Ready(())
                    };
                }
                std::task::Poll::Ready(Some(event)) => {
                    event.unwrap();
                }
                std::task::Poll::Ready(None) => panic!("review must remain pending"),
            }
        }
    }));
    assert_eq!(fixture.transport.wire.lock().unwrap().len(), 1);
    assert_eq!(fixture.preparer.active.load(Ordering::SeqCst), 1);
    assert!(
        fixture
            .authority
            .pending
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .is_live()
    );
    drop(turn);
    assert_eq!(fixture.preparer.active.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.transport.drops.load(Ordering::SeqCst), 1);
    assert!(
        !fixture
            .authority
            .pending
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .is_live()
    );
    assert_eq!(fixture.write_count(), 0);
}

#[test]
fn preparation_permit_is_bounded_and_returned_on_drop() {
    let active = Arc::new(AtomicUsize::new(0));
    let mut permits: Vec<_> = (0..MAX_NATIVE_MCP_PERMISSION_PREPARATIONS)
        .map(|_| Permit::acquire(&active).unwrap())
        .collect();
    assert!(Permit::acquire(&active).is_err());
    permits.pop();
    assert!(Permit::acquire(&active).is_ok());
    drop(permits);
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[test]
fn oversized_complete_review_evidence_is_denied_without_omitting_schema() {
    let schema =
        serde_json::to_string(&json!({"type":"object", "description":"x".repeat(17000)})).unwrap();
    for mode in [PermissionMode::Auto, PermissionMode::Ask] {
        let fixture = Fixture::new(
            mode,
            "allow",
            PermissionPromptDecision::AllowOnce,
            &schema,
            vec![json!({})],
            NativeConfiguredPermissionRules::default(),
        );
        fixture.run();
        assert_eq!(
            fixture.write_count(),
            usize::from(mode == PermissionMode::Ask)
        );
        assert!(fixture.transport.wire.lock().unwrap().is_empty());
    }
}

#[test]
fn actual_registered_builtin_lookup_is_inert_exact_and_rejects_duplicates() {
    use machine_god_core::*;
    struct Named;
    impl Tool for Named {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: ToolName::new("mcp_explicit_builtin").unwrap(),
                description: "fixture".into(),
                input_schema: json!({"type":"object"}),
            }
        }
        fn execute(
            &self,
            _: ToolContext,
            _: serde_json::Value,
            _: CancellationToken,
        ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
            panic!("no effects")
        }
    }
    let named: Arc<dyn Tool> = Arc::new(Named);
    let authority = |count| {
        Arc::new(
            NativePermissionTargetAuthority::new(
                std::fs::File::open("/").unwrap(),
                "/".into(),
                (0..count)
                    .map(|_| NativePermissionTargetTool::Ordinary(named.clone()))
                    .collect(),
            )
            .unwrap(),
        )
    };
    let name = ToolName::new("mcp_explicit_builtin").unwrap();
    assert!(authority(1).has_registered_tool(&name).unwrap());
    assert!(!authority(0).has_registered_tool(&name).unwrap());
    assert!(authority(2).has_registered_tool(&name).is_err());
    let fixture = standard(PermissionMode::Ask, "allow");
    // The fixture has not installed a session turn. A real builtin allocation
    // must dispatch before consulting MCP contexts, regardless of its spelling.
    let preparer = NativeMcpPermissionPreparer::new(
        authority(1),
        fixture.builtin.clone(),
        fixture.authority.clone(),
        fixture.preparer.contexts.clone(),
        fixture.preparer.review_contexts.clone(),
        fixture.preparer.reviewer.clone(),
        "/workspace",
    )
    .unwrap();
    let call_id = ToolCallId::new("call").unwrap();
    let arguments = json!({});
    let request = PermissionRequest {
        id: PermissionRequestId::new("request").unwrap(),
        session_id: SessionId::new("session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("life").unwrap(),
        turn_id: TurnId::new("turn").unwrap(),
        capability: Capability::Tool {
            name: name.clone(),
            call_id: call_id.clone(),
            arguments: arguments.clone(),
        },
        risk: PermissionRisk::Low,
        reason: "hint only".into(),
    };
    let invocation = PermissionInvocation {
        tool_name: &name,
        call_id: &call_id,
        arguments: &arguments,
    };
    drop(preparer.prepare(&request, invocation, CancellationToken::new()));
    assert_eq!(fixture.builtin.calls.load(Ordering::SeqCst), 0);
    assert!(block_on(preparer.prepare(&request, invocation, CancellationToken::new())).is_err());
    assert_eq!(fixture.builtin.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.authority.calls.load(Ordering::SeqCst), 0);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert!(block_on(preparer.prepare(&request, invocation, cancellation)).is_err());
    assert_eq!(fixture.builtin.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn exact_context_close_retires_only_selected_turn_and_preserves_registration_exclusivity() {
    use machine_god_core::*;
    use machine_god_testkit::{
        InMemorySessionStore, ScriptedModelProvider, ScriptedPermissionHandler,
    };
    let contexts = NativeMcpContexts::new();
    let engine = Engine::builder()
        .session_store(InMemorySessionStore::default())
        .provider(ScriptedModelProvider::new("fixture", []))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let sessions: Vec<_> = (0..2)
        .map(|index| {
            engine
                .create_session(
                    SessionId::new(format!("session-{index}")).unwrap(),
                    SessionIncarnationId::new("life").unwrap(),
                )
                .unwrap()
        })
        .collect();
    let owners: Vec<_> = sessions
        .iter()
        .map(|session| contexts.register(session).unwrap())
        .collect();
    let turns: Vec<_> = sessions
        .iter()
        .map(|session| block_on(session.prompt("request")).unwrap())
        .collect();
    let _registrations: Vec<_> = owners
        .iter()
        .zip(&sessions)
        .zip(&turns)
        .map(|((owner, session), turn)| owner.begin(session, turn).unwrap())
        .collect();
    let lookup = |index: usize| ToolContext {
        session_id: sessions[index].id(),
        session_incarnation_id: sessions[index].incarnation_id(),
        turn_id: turns[index].id().clone(),
        call_id: ToolCallId::new("call").unwrap(),
    };
    let first = contexts.snapshot_for_tool(&lookup(0)).unwrap();
    let second = contexts.snapshot_for_tool(&lookup(1)).unwrap();
    contexts.close_turn(
        &sessions[0].id(),
        &sessions[1].incarnation_id(),
        &TurnId::new("foreign-turn").unwrap(),
    );
    assert!(first.is_live() && second.is_live());
    contexts.close_turn(
        &sessions[0].id(),
        &sessions[0].incarnation_id(),
        turns[0].id(),
    );
    assert!(!first.is_live());
    assert!(second.is_live());
    assert!(owners[0].begin(&sessions[0], &turns[0]).is_err());
    let _ = turns[1].handle().cancel();
    contexts.close_turn(
        &sessions[1].id(),
        &sessions[1].incarnation_id(),
        turns[1].id(),
    );
    assert!(owners[1].begin(&sessions[1], &turns[1]).is_err());
}

#[test]
fn injected_workspace_scope_never_falls_back_and_drives_review_presentation() {
    let missing = Fixture::with_workspace(
        PermissionMode::Yolo,
        "allow",
        PermissionPromptDecision::AllowOnce,
        r#"{"type":"object"}"#,
        vec![json!({})],
        NativeConfiguredPermissionRules::default(),
        Some((Arc::new(NativeWorkspaceContexts::new()), None)),
    );
    missing.run();
    assert_eq!(missing.write_count(), 0);
    assert_eq!(missing.authority.calls.load(Ordering::SeqCst), 0);
    let primary = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    let expected_workspace = primary.to_str().unwrap().to_owned();
    let state = std::fs::canonicalize("/etc").unwrap();
    let authority = NativeWorkspaceAuthority::open_blocking(
        std::fs::File::open(&primary).unwrap().into(),
        primary,
        Some(std::fs::File::open(&state).unwrap().into()),
        state,
        vec![],
        false,
    )
    .unwrap();
    let fixture = Fixture::with_workspace(
        PermissionMode::Auto,
        "allow",
        PermissionPromptDecision::Deny,
        r#"{"type":"object"}"#,
        vec![json!({})],
        NativeConfiguredPermissionRules::default(),
        Some((Arc::new(NativeWorkspaceContexts::new()), Some(authority))),
    );
    fixture.run();
    assert_eq!(fixture.write_count(), 1);
    let wire = serde_json::to_string(&fixture.transport.wire.lock().unwrap()[0]).unwrap();
    assert!(wire.contains(&expected_workspace));
    assert!(!wire.contains("/workspace"));
}

#[test]
fn typed_request_marker_rejects_foreign_id_and_repeated_attachment() {
    use crate::mcp::{protocol::RpcId, submission::McpPendingToolReservation};
    use machine_god_core::{ToolCallId, ToolName};
    let authority = Authority::new(r#"{"type":"object"}"#);
    let name = ToolName::new(NAME).unwrap();
    let call = ToolCallId::new("call").unwrap();
    let arguments = json!({});
    let invocation = PermissionInvocation {
        tool_name: &name,
        call_id: &call,
        arguments: &arguments,
    };
    let (wrong, lease) = McpPendingToolReservation::leased(RpcId::Integer(43));
    assert!(
        authority
            .project(invocation)
            .unwrap()
            .with_reservation(lease)
            .is_err()
    );
    assert!(!wrong.is_live());
    let (first, lease) = McpPendingToolReservation::leased(RpcId::Integer(42));
    let request = authority
        .project(invocation)
        .unwrap()
        .with_reservation(lease)
        .unwrap();
    let (second, lease) = McpPendingToolReservation::leased(RpcId::Integer(42));
    assert!(request.with_reservation(lease).is_err());
    assert!(!first.is_live() && !second.is_live());
}
