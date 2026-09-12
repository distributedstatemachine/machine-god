use super::*;

fn required(name: &str) -> crate::mcp::config::McpConfig {
    crate::mcp::config::McpConfig::decode(
        format!(r#"{{"mcp":{{"{name}":{{"command":"unexecuted","required":true}}}}}}"#).as_bytes(),
    )
    .unwrap()
}

#[test]
fn required_readiness_observes_exact_routes_without_taking_operation_mutex() {
    let runtime = standalone();
    let candidate = candidate(&runtime, "calendar", &[], Arc::default());
    let checkpoint = candidate.publication_checkpoint();
    let route = candidate.publication.servers[0].clone();
    runtime.publish(candidate).unwrap();
    let mut peer = route.peer.try_lock().unwrap();
    assert!(
        runtime
            .required_readiness(&checkpoint, &required("calendar"))
            .is_ok()
    );
    assert!(
        runtime
            .required_readiness(&checkpoint, &required("missing"))
            .is_err()
    );
    peer.close();
    assert!(
        runtime
            .required_readiness(&checkpoint, &required("calendar"))
            .is_err()
    );
}

#[test]
fn required_readiness_rejects_foreign_replaced_closed_and_disabled_views() {
    let runtime = standalone();
    let candidate = candidate(&runtime, "calendar", &[], Arc::default());
    let checkpoint = candidate.publication_checkpoint();
    runtime.publish(candidate).unwrap();
    let disabled = crate::mcp::config::McpConfig::decode(
        br#"{"mcp":{"calendar":{"command":"unexecuted","required":true,"enabled":false}}}"#,
    )
    .unwrap();
    assert!(runtime.required_readiness(&checkpoint, &disabled).is_err());
    assert!(
        standalone()
            .required_readiness(&checkpoint, &required("calendar"))
            .is_err()
    );
    runtime
        .publish(super::candidate(&runtime, "calendar", &[], Arc::default()))
        .unwrap();
    assert!(
        runtime
            .required_readiness(&checkpoint, &required("calendar"))
            .is_err()
    );
    let current = runtime.publication_checkpoint().unwrap();
    assert!(
        runtime
            .required_readiness(&current, &required("calendar"))
            .is_ok()
    );
    runtime.close();
    assert!(
        runtime
            .required_readiness(&current, &required("calendar"))
            .is_err()
    );
}

#[test]
fn required_readiness_rechecks_actual_authority_without_blocking_on_optional_peers() {
    let runtime = standalone();
    let candidate = candidate(&runtime, "calendar", &[], Arc::default());
    let checkpoint = candidate.publication_checkpoint();
    let route = candidate.publication.servers[0].clone();
    runtime.publish(candidate).unwrap();
    route.cancellation.cancel();
    assert!(
        runtime
            .required_readiness(&checkpoint, &required("calendar"))
            .is_err()
    );
    assert!(
        runtime
            .required_readiness(&checkpoint, &crate::mcp::config::McpConfig::default())
            .is_ok()
    );
}
