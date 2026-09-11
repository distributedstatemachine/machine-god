use super::*;

fn guarded_candidate(
    runtime: &NativeMcpRuntime,
    feature_only: bool,
    guards: Arc<[CancellationToken]>,
    epoch: Instant,
) -> NativeMcpRuntimeCandidate {
    let mut peer = script::ScriptPeer::new(Arc::default());
    peer.tools = !feature_only;
    let (kind, result) = if feature_only {
        (
            McpCatalogKind::Resources,
            json!({"resultType":"complete","resources":[{"name":"guide","uri":"custom://guide"}]}),
        )
    } else {
        (
            McpCatalogKind::Tools,
            json!({"resultType":"complete","tools":[{"name":"lookup","inputSchema":{"type":"object"}}]}),
        )
    };
    let mut builder =
        McpCatalogBuilder::new(kind, ProtocolVersion::Modern, McpCatalogLimits::default()).unwrap();
    let wire = serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"result":result})).unwrap();
    builder
        .append_response(&wire, &RpcId::Integer(1), None, 17)
        .unwrap();
    let catalog =
        McpDescriptorCatalog::admit(builder.finish().unwrap(), McpDescriptorLimits::default())
            .unwrap();
    runtime
        .prepare_candidate(
            vec![NativeMcpServerCandidate {
                server: Arc::from("replacement"),
                configuration: Arc::from(&b"configuration"[..]),
                authentication: Arc::from(&b"authentication"[..]),
                catalogs: vec![catalog],
                catalog_epoch: epoch,
                operation_timeout: Duration::from_secs(120),
                authority_cancellations: guards,
                peer: NativeMcpOwnedPeer::Script(peer),
            }],
            &[],
        )
        .unwrap()
}

#[test]
fn authority_revocation_between_prepare_and_publish_preserves_the_active_runtime() {
    for feature_only in [false, true] {
        for index in 0..crate::mcp::submission::MAX_MCP_RUNTIME_CANCELLATION_GUARDS {
            let runtime = standalone();
            runtime
                .publish(candidate(&runtime, "calendar", &["lookup"], Arc::default()))
                .unwrap();
            let original = runtime.state.lock().unwrap().active.clone().unwrap();
            let guards: Arc<[CancellationToken]> = (0
                ..crate::mcp::submission::MAX_MCP_RUNTIME_CANCELLATION_GUARDS)
                .map(|_| CancellationToken::new())
                .collect();
            let prepared =
                guarded_candidate(&runtime, feature_only, guards.clone(), Instant::now());
            assert_eq!(prepared.publication.tools.is_empty(), feature_only);
            guards[index].cancel();
            assert_eq!(
                runtime.publish(prepared),
                Err(NativeMcpRuntimeError::Cancelled)
            );
            let state = runtime.state.lock().unwrap();
            assert!(Arc::ptr_eq(state.active.as_ref().unwrap(), &original));
            assert!(state.retired.is_empty());
            assert_eq!(state.retired_byte_charge, 0);
            assert!(original.check().is_ok());
            assert!(
                original
                    .tools
                    .values()
                    .all(|tool| tool.binding.live().is_ok())
            );
        }
    }
}

#[test]
fn route_retirement_also_rejects_a_prepared_feature_only_server() {
    let runtime = standalone();
    let prepared = guarded_candidate(&runtime, true, Arc::from([]), Instant::now());
    prepared.publication.servers[0].cancellation.cancel();
    assert_eq!(
        runtime.publish(prepared),
        Err(NativeMcpRuntimeError::Cancelled)
    );
    assert!(runtime.state.lock().unwrap().active.is_none());
}

#[test]
fn catalog_origin_survives_candidate_publication_without_becoming_current_time() {
    let runtime = standalone();
    let epoch = Instant::now()
        .checked_sub(Duration::from_secs(3600))
        .unwrap();
    let prepared = guarded_candidate(&runtime, true, Arc::from([]), epoch);
    assert_eq!(prepared.catalog_epoch("replacement"), Some(epoch));
    assert_eq!(prepared.catalog_epoch("Replacement"), None);
    runtime.publish(prepared).unwrap();
    assert_eq!(
        runtime
            .state
            .lock()
            .unwrap()
            .active
            .as_ref()
            .unwrap()
            .servers[0]
            .catalog_epoch,
        epoch
    );
}
