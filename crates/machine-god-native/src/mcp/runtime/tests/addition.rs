use super::*;
use futures_executor::block_on;
use machine_god_testkit::{InMemorySessionStore, ScriptedModelProvider, ScriptedPermissionHandler};
use std::sync::atomic::AtomicBool;

pub(super) fn server(name: &str, tools: &[&str]) -> NativeMcpServerCandidate {
    let mut builder = McpCatalogBuilder::new(
        McpCatalogKind::Tools,
        ProtocolVersion::Modern,
        McpCatalogLimits::default(),
    )
    .unwrap();
    let items: Vec<_> = tools
        .iter()
        .map(|name| json!({"name":name,"inputSchema":{"type":"object"}}))
        .collect();
    let wire = serde_json::to_vec(
        &json!({"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","tools":items}}),
    )
    .unwrap();
    builder
        .append_response(&wire, &RpcId::Integer(1), None, 19)
        .unwrap();
    NativeMcpServerCandidate {
        server: Arc::from(name),
        configuration: Arc::from(&b"configuration"[..]),
        authentication: Arc::from(&b"authentication"[..]),
        catalogs: vec![
            McpDescriptorCatalog::admit(builder.finish().unwrap(), McpDescriptorLimits::default())
                .unwrap(),
        ],
        catalog_epoch: Instant::now(),
        refresh: None,
        peer: NativeMcpOwnedPeer::Script(script::ScriptPeer::new(Arc::default())),
        operation_timeout: Duration::from_secs(120),
        authority_cancellations: Arc::from([]),
    }
}

fn closed(server: &NativeMcpServerCandidate) -> Arc<AtomicBool> {
    let NativeMcpOwnedPeer::Script(peer) = &server.peer else {
        unreachable!()
    };
    peer.closed.clone()
}

fn install(runtime: &NativeMcpRuntime, server: NativeMcpServerCandidate) {
    runtime
        .publish(runtime.prepare_candidate(vec![server], &[]).unwrap())
        .unwrap();
}

fn active(runtime: &NativeMcpRuntime) -> Arc<super::super::candidate::Publication> {
    runtime.state.lock().unwrap().active.clone().unwrap()
}

fn names(publication: &super::super::candidate::Publication) -> Vec<String> {
    publication
        .snapshot
        .tools()
        .iter()
        .map(|tool| tool.name().to_owned())
        .collect()
}

pub(super) fn conversation(
    runtime: &NativeMcpRuntime,
    id: &str,
) -> (
    Engine,
    NativeConversation,
    NativeConversationTurn,
    ToolContext,
) {
    let engine = Engine::builder()
        .session_store(InMemorySessionStore::default())
        .provider(ScriptedModelProvider::new("fixture", []))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = engine
        .create_session(
            SessionId::new(id).unwrap(),
            SessionIncarnationId::new("life").unwrap(),
        )
        .unwrap();
    let conversation = NativeConversation::from_session(session)
        .unwrap()
        .with_mcp_contexts(&runtime.contexts)
        .unwrap();
    let turn = block_on(conversation.prompt("requested change".into(), 1)).unwrap();
    let context = ToolContext {
        session_id: conversation.id(),
        session_incarnation_id: conversation.incarnation_id(),
        turn_id: turn.handle().id().clone(),
        call_id: ToolCallId::new("call").unwrap(),
    };
    (engine, conversation, turn, context)
}

#[test]
fn addition_preserves_old_turn_view_and_exact_routes_while_new_turn_sees_union() {
    let runtime = standalone();
    let required = server("required", &["lookup"]);
    let required_closed = closed(&required);
    install(&runtime, required);
    let (_engine, _conversation, _turn, context) = conversation(&runtime, "old");
    let native = runtime.contexts.snapshot_for_tool(&context).unwrap();
    let old = runtime
        .for_turn(&native.registry().unwrap())
        .unwrap()
        .unwrap();
    let old_tool = old.tools.values().next().unwrap().clone();
    let old_registration = old.snapshot.tools()[0].executable().unwrap();
    let weak = Arc::downgrade(&old);
    drop(old);
    let expected = runtime.publication_checkpoint().unwrap();
    let addition = runtime
        .prepare_addition(vec![server("optional", &["extra"])], &[], &expected)
        .unwrap();
    runtime.publish_addition(addition).unwrap();
    let old = weak
        .upgrade()
        .expect("active lineage retains original view");
    let combined = active(&runtime);
    assert_eq!(names(&old), ["mcp_required_lookup"]);
    assert_eq!(
        names(&combined),
        ["mcp_required_lookup", "mcp_optional_extra"]
    );
    assert!(Arc::ptr_eq(
        &old,
        &runtime
            .for_turn(&native.registry().unwrap())
            .unwrap()
            .unwrap()
    ));
    assert!(Arc::ptr_eq(
        &old_tool,
        combined.tools.get(&old_tool.name).unwrap()
    ));
    assert!(Arc::ptr_eq(
        &old_registration,
        &combined.snapshot.tools()[0].executable().unwrap()
    ));
    assert!(Arc::ptr_eq(&old.servers[0], &combined.servers[0]));
    assert!(Arc::ptr_eq(&old.retired, &combined.retired));
    assert_eq!(combined.descriptors.len(), 2);
    assert_eq!(combined.descriptors[0].servers()[0].name(), "required");
    assert_eq!(combined.descriptors[1].servers()[0].name(), "optional");
    let (_new_engine, _new_conversation, _new_turn, context) = conversation(&runtime, "new");
    let new = runtime.contexts.snapshot_for_tool(&context).unwrap();
    assert!(Arc::ptr_eq(
        &combined,
        &runtime.for_turn(&new.registry().unwrap()).unwrap().unwrap()
    ));
    assert!(!required_closed.load(Ordering::Acquire));
    assert!(old_tool.binding.live().is_ok());
    assert!(runtime.state.lock().unwrap().retired.is_empty());
}

#[test]
fn addition_reserves_existing_names_and_builtins_without_renaming_old_tools() {
    let runtime = standalone();
    let first = "a".repeat(70);
    let second = format!("{first}b");
    install(&runtime, server(&first, &["lookup"]));
    let old = active(&runtime);
    let old_name = names(&old).remove(0);
    let reserved = format!("{}_2", &old_name[..62]);
    let expected = runtime.publication_checkpoint().unwrap();
    let addition = runtime
        .prepare_addition(vec![server(&second, &["lookup"])], &[&reserved], &expected)
        .unwrap();
    runtime.publish_addition(addition).unwrap();
    assert_eq!(
        names(&active(&runtime)),
        [old_name.clone(), format!("{}_3", &old_name[..62])]
    );
    assert!(Arc::ptr_eq(
        old.tools.values().next().unwrap(),
        active(&runtime)
            .tools
            .get(&ToolName::new(old_name).unwrap())
            .unwrap()
    ));
}

#[test]
fn addition_revalidates_each_existing_and_new_guard_before_publication() {
    for existing in [false, true] {
        for index in 0..crate::mcp::submission::MAX_MCP_RUNTIME_CANCELLATION_GUARDS {
            let runtime = standalone();
            let guards: Arc<[CancellationToken]> =
                (0..8).map(|_| CancellationToken::new()).collect();
            let mut required = server("required", &["lookup"]);
            let mut optional = server("optional", &["extra"]);
            if existing {
                required.authority_cancellations = guards.clone();
            } else {
                optional.authority_cancellations = guards.clone();
            }
            let old_closed = closed(&required);
            let new_closed = closed(&optional);
            install(&runtime, required);
            let old = active(&runtime);
            let addition = runtime
                .prepare_addition(
                    vec![optional],
                    &[],
                    &runtime.publication_checkpoint().unwrap(),
                )
                .unwrap();
            guards[index].cancel();
            assert_eq!(
                runtime.publish_addition(addition),
                Err(NativeMcpRuntimeError::Cancelled)
            );
            assert!(Arc::ptr_eq(&old, &active(&runtime)));
            assert!(!old.retired.load(Ordering::Acquire));
            assert!(!old_closed.load(Ordering::Acquire));
            assert!(new_closed.load(Ordering::Acquire));
            assert!(runtime.state.lock().unwrap().retired.is_empty());
        }
    }
}

#[test]
fn empty_absent_foreign_duplicate_and_second_additions_are_explicitly_rejected() {
    let runtime = standalone();
    let expected = runtime.publication_checkpoint().unwrap();
    assert!(matches!(
        runtime.prepare_addition(vec![], &[], &expected),
        Err(NativeMcpRuntimeError::Invalid)
    ));
    assert!(matches!(
        runtime.prepare_addition(vec![server("optional", &[])], &[], &expected),
        Err(NativeMcpRuntimeError::Unavailable)
    ));
    install(&runtime, server("required", &[]));
    let expected = runtime.publication_checkpoint().unwrap();
    let foreign = standalone().publication_checkpoint().unwrap();
    assert!(matches!(
        runtime.prepare_addition(vec![server("optional", &[])], &[], &foreign),
        Err(NativeMcpRuntimeError::Invalid)
    ));
    assert!(matches!(
        runtime.prepare_addition(vec![server("required", &[])], &[], &expected),
        Err(NativeMcpRuntimeError::Invalid)
    ));
    assert!(matches!(
        runtime.prepare_addition(
            vec![server("optional", &[]), server("optional", &[])],
            &[],
            &expected
        ),
        Err(NativeMcpRuntimeError::Invalid)
    ));
    let addition = runtime
        .prepare_addition(vec![server("optional", &[])], &[], &expected)
        .unwrap();
    runtime.publish_addition(addition).unwrap();
    assert!(matches!(
        runtime.prepare_addition(
            vec![server("later", &[])],
            &[],
            &runtime.publication_checkpoint().unwrap()
        ),
        Err(NativeMcpRuntimeError::Limit)
    ));
}

#[test]
fn competing_additions_have_one_winner_and_stale_candidate_drops_only_new_peers() {
    let runtime = standalone();
    let required = server("required", &["lookup"]);
    let old_closed = closed(&required);
    install(&runtime, required);
    let expected = runtime.publication_checkpoint().unwrap();
    let first = runtime
        .prepare_addition(vec![server("first", &[])], &[], &expected)
        .unwrap();
    let second = server("second", &[]);
    let second_closed = closed(&second);
    let second = runtime
        .prepare_addition(vec![second], &[], &expected)
        .unwrap();
    runtime.publish_addition(first).unwrap();
    let current = active(&runtime);
    assert_eq!(
        runtime.publish_addition(second),
        Err(NativeMcpRuntimeError::Unavailable)
    );
    assert!(Arc::ptr_eq(&current, &active(&runtime)));
    assert!(!old_closed.load(Ordering::Acquire));
    assert!(second_closed.load(Ordering::Acquire));
    assert!(matches!(
        runtime.prepare_addition(vec![server("third", &[])], &[], &expected),
        Err(NativeMcpRuntimeError::Unavailable)
    ));
}

#[test]
fn merged_tool_server_and_byte_limits_reject_without_altering_active_routes() {
    for limit in ["servers", "tools", "bytes"] {
        let mut runtime = standalone();
        install(&runtime, server("required", &["lookup"]));
        let old = active(&runtime);
        let expected = runtime.publication_checkpoint().unwrap();
        match limit {
            "servers" => runtime.limits.max_servers = 1,
            "tools" => runtime.limits.max_tools = 1,
            _ => runtime.limits.max_retained_bytes = old.retained_bytes * 2,
        }
        assert!(matches!(
            runtime.prepare_addition(vec![server("optional", &["extra"])], &[], &expected),
            Err(NativeMcpRuntimeError::Limit)
        ));
        assert!(Arc::ptr_eq(&old, &active(&runtime)));
        assert!(old.tools.values().all(|tool| tool.binding.live().is_ok()));
        assert!(runtime.state.lock().unwrap().retired.is_empty());
    }
}

#[test]
fn final_byte_recheck_preserves_active_and_retired_ownership() {
    let mut runtime = standalone();
    install(&runtime, server("retired", &["prior"]));
    install(&runtime, server("required", &["lookup"]));
    let old = active(&runtime);
    let addition = runtime
        .prepare_addition(
            vec![server("optional", &["extra"])],
            &[],
            &runtime.publication_checkpoint().unwrap(),
        )
        .unwrap();
    runtime.limits.max_retained_bytes = addition.retained_byte_charge();
    assert_eq!(
        runtime.publish_addition(addition),
        Err(NativeMcpRuntimeError::Limit)
    );
    assert!(Arc::ptr_eq(&old, &active(&runtime)));
    assert_eq!(runtime.state.lock().unwrap().retired.len(), 1);
    assert!(old.tools.values().all(|tool| tool.binding.live().is_ok()));
}

#[test]
fn full_replacement_retires_both_views_and_drains_each_unique_peer_once() {
    for close in [false, true] {
        let runtime = standalone();
        let required = server("required", &["lookup"]);
        let required_closed = closed(&required);
        install(&runtime, required);
        let old = Arc::downgrade(&active(&runtime));
        let optional = server("optional", &["extra"]);
        let optional_closed = closed(&optional);
        let addition = runtime
            .prepare_addition(
                vec![optional],
                &[],
                &runtime.publication_checkpoint().unwrap(),
            )
            .unwrap();
        runtime.publish_addition(addition).unwrap();
        let combined = active(&runtime);
        let charge = combined.retained_bytes;
        let prior = old.upgrade().unwrap();
        if close {
            runtime.close();
        } else {
            install(&runtime, server("replacement", &[]));
        }
        assert!(prior.check().is_err());
        assert!(combined.check().is_err());
        assert!(
            combined
                .tools
                .values()
                .all(|tool| tool.binding.live().is_err())
        );
        assert_eq!(runtime.state.lock().unwrap().retired.len(), 2);
        assert_eq!(runtime.state.lock().unwrap().retired_byte_charge, charge);
        drop(prior);
        drop(combined);
        assert!(old.upgrade().is_none());
        let receipts = block_on(runtime.drain_retired(
            Instant::now() + Duration::from_secs(1),
            CancellationToken::new(),
        ))
        .unwrap();
        assert_eq!(receipts.len(), 2);
        assert!(receipts.iter().all(NativeMcpPeerCompletion::is_complete));
        assert!(required_closed.load(Ordering::Acquire));
        assert!(optional_closed.load(Ordering::Acquire));
        assert_eq!(runtime.state.lock().unwrap().retired_byte_charge, 0);
    }
}

#[test]
fn original_feature_witness_survives_addition_but_not_whole_replacement() {
    let runtime = Arc::new(standalone());
    let mut required = server("required", &[]);
    required.catalog_epoch = runtime.clock.now();
    required.peer = NativeMcpOwnedPeer::Script(script::ScriptPeer::new(Arc::default()).with_response(|id| {
        format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"resultType":"complete","resources":[]}}}}"#).into_bytes().into()
    }));
    install(&runtime, required);
    let command = runtime.human_command();
    let request = crate::mcp::control::tests::request("resource list required");
    let result = block_on(command.feature(&request, CancellationToken::new())).unwrap();
    let addition = runtime
        .prepare_addition(
            vec![server("optional", &[])],
            &[],
            &runtime.publication_checkpoint().unwrap(),
        )
        .unwrap();
    runtime.publish_addition(addition).unwrap();
    result.revalidate().unwrap();
    install(&runtime, server("replacement", &[]));
    assert!(result.revalidate().is_err());
}

#[test]
fn prospective_checkpoints_match_only_their_exact_published_view() {
    let runtime = standalone();
    let empty = runtime.publication_checkpoint().unwrap();
    let initial = runtime
        .prepare_candidate(vec![server("required", &[])], &[])
        .unwrap();
    let predicted = initial.publication_checkpoint();
    assert!(
        predicted
            .check(&runtime, &runtime.state.lock().unwrap())
            .is_err()
    );
    runtime.publish_if(initial, &empty).unwrap();
    predicted
        .check(&runtime, &runtime.state.lock().unwrap())
        .unwrap();
    let addition = runtime
        .prepare_addition(vec![server("optional", &[])], &[], &predicted)
        .unwrap();
    let appended = addition.publication_checkpoint();
    assert!(
        appended
            .check(&runtime, &runtime.state.lock().unwrap())
            .is_err()
    );
    runtime.publish_addition(addition).unwrap();
    appended
        .check(&runtime, &runtime.state.lock().unwrap())
        .unwrap();
    assert!(
        predicted
            .check(&runtime, &runtime.state.lock().unwrap())
            .is_err()
    );
    install(&runtime, server("replacement", &[]));
    assert!(
        appended
            .check(&runtime, &runtime.state.lock().unwrap())
            .is_err()
    );
    let foreign = standalone();
    assert!(matches!(
        appended.check(&foreign, &foreign.state.lock().unwrap()),
        Err(NativeMcpRuntimeError::Invalid)
    ));
    // A full replacement starts a fresh one-addition lineage.
    runtime
        .publish_addition(
            runtime
                .prepare_addition(
                    vec![server("new_optional", &[])],
                    &[],
                    &runtime.publication_checkpoint().unwrap(),
                )
                .unwrap(),
        )
        .unwrap();
}

#[test]
fn foreign_publication_and_abandoned_additions_never_cancel_the_original() {
    let runtime = standalone();
    let required = server("required", &["lookup"]);
    let old_closed = closed(&required);
    install(&runtime, required);
    let current = active(&runtime);
    for foreign in [false, true] {
        let new = server("optional", &["extra"]);
        let new_closed = closed(&new);
        let addition = runtime
            .prepare_addition(vec![new], &[], &runtime.publication_checkpoint().unwrap())
            .unwrap();
        if foreign {
            assert_eq!(
                standalone().publish_addition(addition),
                Err(NativeMcpRuntimeError::Invalid)
            );
        } else {
            drop(addition);
        }
        assert!(Arc::ptr_eq(&current, &active(&runtime)));
        assert!(!old_closed.load(Ordering::Acquire));
        assert!(new_closed.load(Ordering::Acquire));
        assert!(
            current
                .tools
                .values()
                .all(|tool| tool.binding.live().is_ok())
        );
    }
    let addition = runtime
        .prepare_addition(
            vec![server("optional", &[])],
            &[],
            &runtime.publication_checkpoint().unwrap(),
        )
        .unwrap();
    runtime.close();
    assert_eq!(
        runtime.publish_addition(addition),
        Err(NativeMcpRuntimeError::Unavailable)
    );
    assert_eq!(runtime.state.lock().unwrap().retired.len(), 1);
}
