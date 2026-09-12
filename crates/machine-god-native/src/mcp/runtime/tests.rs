use super::*;
use crate::{
    mcp::{
        catalog::{McpDescriptorCatalog, McpDescriptorLimits},
        pagination::{McpCatalogBuilder, McpCatalogKind, McpCatalogLimits},
        protocol::{ProtocolVersion, RpcId},
    },
    *,
};
use machine_god_core::*;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

mod addition;
mod checkpoint;
mod execution;
mod features;
mod fixture;
mod lifecycle;
mod publication;
pub(super) mod script;
use fixture::*;

#[test]
fn retired_generation_bytes_remain_charged_until_cleanup_drains() {
    let mut runtime = standalone();
    let first = candidate(&runtime, "calendar", &["lookup"], Arc::default());
    let charge = first.retained_byte_charge();
    runtime.limits.max_retained_bytes = charge * 2;
    runtime.publish(first).unwrap();
    runtime
        .publish(candidate(&runtime, "calendar", &["lookup"], Arc::default()))
        .unwrap();
    let active = runtime.state.lock().unwrap().active.clone().unwrap();
    assert_eq!(
        runtime.publish(candidate(&runtime, "calendar", &["lookup"], Arc::default())),
        Err(NativeMcpRuntimeError::Limit)
    );
    assert!(
        active
            .tools
            .values()
            .all(|tool| tool.binding.live().is_ok())
    );
    futures_executor::block_on(runtime.drain_retired(
        Instant::now() + Duration::from_secs(1),
        CancellationToken::new(),
    ))
    .unwrap();
    runtime
        .publish(candidate(&runtime, "calendar", &["lookup"], Arc::default()))
        .unwrap();
}

#[test]
fn feature_only_server_does_not_require_an_unadvertised_tools_catalog() {
    let runtime = standalone();
    let mut peer = script::ScriptPeer::new(Arc::default());
    peer.tools = false;
    let mut builder = McpCatalogBuilder::new(
        McpCatalogKind::Resources,
        ProtocolVersion::Modern,
        McpCatalogLimits::default(),
    )
    .unwrap();
    builder.append_response(br#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","resources":[{"name":"guide","uri":"custom://guide"}]}}"#, &RpcId::Integer(1), None, 0).unwrap();
    let catalog =
        McpDescriptorCatalog::admit(builder.finish().unwrap(), McpDescriptorLimits::default())
            .unwrap();
    let candidate = runtime
        .prepare_candidate(
            vec![NativeMcpServerCandidate {
                server: Arc::from("reference"),
                configuration: Arc::from(&b"configuration"[..]),
                authentication: Arc::from(&b"credential"[..]),
                catalogs: vec![catalog],
                catalog_epoch: Instant::now(),
                peer: NativeMcpOwnedPeer::Script(peer),
                operation_timeout: std::time::Duration::from_secs(120),
                authority_cancellations: Arc::from([]),
            }],
            &[],
        )
        .unwrap();
    assert!(candidate.descriptors().tools().is_empty());
    runtime.publish(candidate).unwrap();
}

#[test]
fn selected_dynamic_tool_uses_actual_auto_controller_exact_proof_and_archive_effect() {
    let arguments = machine_god_core::json::from_str(r#"{"n":9007199254740993.0000001,"tiny":1e-99999,"zero":-0,"$serde_json::private::Number":{"$serde_json::private::RawValue":"literal"}}"#).unwrap();
    let fixture = Fixture::new(std::slice::from_ref(&arguments), PermissionMode::Auto, 0);
    let events = fixture.run();
    assert_eq!(fixture.executor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.executor.inputs.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.transport.reviews.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.executor.values.lock().unwrap()[0], arguments);
    let writes = fixture.writes.lock().unwrap();
    let sent = machine_god_core::json::from_slice(&writes).unwrap();
    assert_eq!(sent["params"]["arguments"], arguments);
    assert!(String::from_utf8_lossy(&writes).contains("1e-99999"));
    assert!(events.iter().any(|event| matches!(&event.payload, TurnEvent::ToolFinished { call_id, output } if call_id.as_str() == "call-0" && output.content.get("jsonrpc").is_some())));
    let requests = fixture.provider.requests();
    let serialized = serde_json::to_string(&requests.last().unwrap().request).unwrap();
    assert!(
        serialized.contains("fixture_archive"),
        "durable projection must reach later provider request"
    );
    assert!(serialized.contains("fixture_input_archive"));
    let selected = fixture.executor.options.lock().unwrap()[0];
    assert_eq!(
        selected,
        crate::mcp::submission::McpToolCallOptions::new(
            crate::mcp::protocol::NegotiatedProtocol {
                version: ProtocolVersion::Modern,
                transport: crate::mcp::protocol::TransportKind::Stdio
            },
            1
        )
        .unwrap()
        .with_progress_token(1)
    );
    assert_eq!(fixture.runtime.state.lock().unwrap().turns.len(), 1);
}

#[test]
fn executor_dropping_unsent_calls_releases_leases_beyond_peer_capacity() {
    let fixture = Fixture::new(
        &vec![json!({"argument":"same"}); 66],
        PermissionMode::Yolo,
        65,
    );
    fixture.run();
    fixture.run();
    fixture.run();
    assert_eq!(fixture.executor.calls.load(Ordering::SeqCst), 66);
    assert_eq!(fixture.transport.reviews.load(Ordering::SeqCst), 0);
    let writes = fixture.writes.lock().unwrap();
    let sent = machine_god_core::json::from_slice(&writes).unwrap();
    assert_eq!(sent["id"], 66);
}

#[test]
fn constructors_contextless_catalog_and_unpolled_futures_have_no_effects() {
    let runtime = standalone();
    let writes: Arc<Mutex<Vec<u8>>> = Arc::default();
    let prepared = candidate(&runtime, "calendar", &["lookup"], writes.clone());
    assert!(writes.lock().unwrap().is_empty());
    let tool = tool::RuntimeTool(prepared.publication.tools.values().next().unwrap().clone());
    runtime.publish(prepared).unwrap();
    assert!(futures_executor::block_on(runtime.snapshot(CancellationToken::new())).is_err());
    let context = ToolContext {
        session_id: SessionId::new("missing").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("missing").unwrap(),
        turn_id: TurnId::new("missing").unwrap(),
        call_id: ToolCallId::new("missing").unwrap(),
    };
    drop(tool.execute_for_turn(context.clone(), json!({}), CancellationToken::new()));
    assert!(writes.lock().unwrap().is_empty());
    assert!(
        futures_executor::block_on(tool.execute_for_turn(
            context,
            json!({}),
            CancellationToken::new()
        ))
        .is_err()
    );
}

fn candidate(
    runtime: &NativeMcpRuntime,
    server: &str,
    names: &[&str],
    writes: Arc<Mutex<Vec<u8>>>,
) -> NativeMcpRuntimeCandidate {
    let items: Vec<_> = names
        .iter()
        .map(|name| json!({"name":name,"inputSchema":{"type":"object"}}))
        .collect();
    let body = format!(
        r#"{{"jsonrpc":"2.0","id":1,"result":{{"resultType":"complete","tools":{}}}}}"#,
        serde_json::to_string(&items).unwrap()
    );
    let mut builder = McpCatalogBuilder::new(
        McpCatalogKind::Tools,
        ProtocolVersion::Modern,
        McpCatalogLimits::default(),
    )
    .unwrap();
    builder
        .append_response(body.as_bytes(), &RpcId::Integer(1), None, 0)
        .unwrap();
    let catalog =
        McpDescriptorCatalog::admit(builder.finish().unwrap(), McpDescriptorLimits::default())
            .unwrap();
    runtime
        .prepare_candidate(
            vec![NativeMcpServerCandidate {
                server: Arc::from(server),
                configuration: Arc::from(&b"configuration"[..]),
                authentication: Arc::from(&b"credential"[..]),
                catalogs: vec![catalog],
                catalog_epoch: Instant::now(),
                peer: NativeMcpOwnedPeer::Script(script::ScriptPeer::new(writes)),
                operation_timeout: std::time::Duration::from_secs(120),
                authority_cancellations: Arc::from([]),
            }],
            &[MCP_SELECT_TOOL_NAME],
        )
        .unwrap()
}

#[test]
fn publication_replacement_invalidates_all_old_bindings_and_keeps_weak_routes() {
    let runtime = standalone();
    let first = candidate(&runtime, "calendar", &["one", "two"], Arc::default());
    let tools: Vec<_> = first.publication.tools.values().cloned().collect();
    let weak_server = tools[0].server.clone();
    runtime.publish(first).unwrap();
    let second = candidate(&runtime, "calendar", &["one"], Arc::default());
    runtime.publish(second).unwrap();
    assert!(tools.iter().all(|tool| tool.binding.live().is_err()));
    assert!(weak_server.upgrade().is_some());
    let receipts = futures_executor::block_on(runtime.drain_retired(
        Instant::now() + Duration::from_secs(1),
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(receipts.len(), 1);
    assert!(receipts.iter().all(NativeMcpPeerCompletion::is_complete));
    assert!(
        weak_server.upgrade().is_none(),
        "old tool allocations must not retain the peer"
    );
}

#[test]
fn foreign_candidate_and_retirement_capacity_preserve_usable_publication() {
    let runtime = standalone_with_limits(NativeMcpRuntimeLimits {
        max_retired_servers: 1,
        ..Default::default()
    });
    runtime
        .publish(candidate(&runtime, "a", &["tool"], Arc::default()))
        .unwrap();
    let foreign = standalone();
    assert_eq!(
        runtime.publish(candidate(&foreign, "b", &["tool"], Arc::default())),
        Err(NativeMcpRuntimeError::Invalid)
    );
    runtime
        .publish(candidate(&runtime, "a", &["tool"], Arc::default()))
        .unwrap();
    let current = runtime.state.lock().unwrap().active.clone().unwrap();
    assert_eq!(
        runtime.publish(candidate(&runtime, "a", &["tool"], Arc::default())),
        Err(NativeMcpRuntimeError::Limit)
    );
    assert!(
        current
            .tools
            .values()
            .all(|tool| tool.binding.live().is_ok())
    );
    assert!(Arc::ptr_eq(
        &current,
        runtime.state.lock().unwrap().active.as_ref().unwrap()
    ));
}
