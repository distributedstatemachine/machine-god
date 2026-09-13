use super::super::{
    candidate::Publication,
    route::{PeerGuard, ServerRoute},
};
use super::*;
use crate::mcp::control::McpFeatureControlAuthority;
use futures_executor::block_on;

mod replacement;

fn active(runtime: &NativeMcpRuntime) -> Arc<Publication> {
    runtime.state.lock().unwrap().active.clone().unwrap()
}
fn tools(names: &[&str]) -> McpDescriptorCatalog {
    addition::server("unused", names).catalogs.remove(0)
}
fn lane(server: &ServerRoute) -> PeerGuard<'_> {
    let authority = McpFeatureControlAuthority::for_human(
        CancellationToken::new(),
        CancellationToken::new(),
        CancellationToken::new(),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
        Arc::from([]),
    )
    .unwrap();
    block_on(server.acquire_feature(&authority)).unwrap()
}
fn install(runtime: &NativeMcpRuntime, names: &[&str]) {
    runtime
        .publish(
            runtime
                .prepare_candidate(vec![addition::server("selected", names)], &[])
                .unwrap(),
        )
        .unwrap();
}
fn refresh(runtime: &NativeMcpRuntime, names: &[&str]) -> Result<()> {
    let view = active(runtime);
    let server = &view.servers[0];
    let mut lane = lane(server);
    let prepared = runtime.prepare_tool_refresh(
        &runtime.publication_checkpoint()?,
        server,
        tools(names),
        &[],
    )?;
    runtime.commit_tool_refresh(prepared, &mut lane)?;
    Ok(())
}

#[test]
fn same_peer_refresh_retires_only_selected_bindings_and_preserves_other_registrations() {
    let runtime = standalone();
    runtime
        .publish(
            runtime
                .prepare_candidate(
                    vec![
                        addition::server("selected", &["old"]),
                        addition::server("other", &["stable"]),
                    ],
                    &[],
                )
                .unwrap(),
        )
        .unwrap();
    let before = active(&runtime);
    let old = before
        .tools
        .values()
        .find(|tool| tool.descriptor.name() == "old")
        .unwrap();
    let stable = before
        .tools
        .values()
        .find(|tool| tool.descriptor.name() == "stable")
        .unwrap();
    let captured = before
        .snapshot
        .tools()
        .iter()
        .find(|tool| tool.server() == "other")
        .unwrap()
        .executable()
        .unwrap();
    refresh(&runtime, &["new"]).unwrap();
    let after = active(&runtime);
    assert!(before.check().is_err());
    assert!(old.binding.live().is_err());
    assert!(stable.binding.live().is_ok());
    assert!(Arc::ptr_eq(stable, after.tools.get(&stable.name).unwrap()));
    assert!(Arc::ptr_eq(
        &captured,
        &after
            .snapshot
            .tools()
            .iter()
            .find(|tool| tool.server() == "other")
            .unwrap()
            .executable()
            .unwrap()
    ));
    assert!(
        before
            .servers
            .iter()
            .zip(&after.servers)
            .all(|(old, new)| Arc::ptr_eq(old, new))
    );
    assert!(runtime.state.lock().unwrap().retired.is_empty());
    assert!(
        after
            .servers
            .iter()
            .all(|server| !server.cancellation.is_cancelled())
    );
    let peer = block_on(after.servers[0].peer.lock());
    let NativeMcpOwnedPeer::Script(peer) = &*peer else {
        panic!()
    };
    assert_eq!(peer.runtimes.len(), 1);
    assert!(Arc::ptr_eq(
        &peer.runtimes[0],
        &after
            .tools
            .values()
            .find(|tool| tool.descriptor.name() == "new")
            .unwrap()
            .binding
    ));
}

#[test]
fn unchanged_catalog_preserves_publication_binding_and_retention_budget() {
    let runtime = standalone();
    install(&runtime, &["stable"]);
    let before = active(&runtime);
    for _ in 0..100 {
        refresh(&runtime, &["stable"]).unwrap();
    }
    assert!(Arc::ptr_eq(&before, &active(&runtime)));
    assert!(
        before
            .tools
            .values()
            .all(|tool| tool.binding.live().is_ok())
    );
    assert!(runtime.state.lock().unwrap().retired_catalogs.is_empty());
}

#[test]
fn refresh_retains_original_builtin_reservations_without_a_controller() {
    let runtime = standalone();
    let unnamed = runtime
        .prepare_candidate(vec![addition::server("selected", &["lookup"])], &[])
        .unwrap();
    let builtin = unnamed.descriptors().tools()[0].name().to_owned();
    drop(unnamed);
    runtime
        .publish(
            runtime
                .prepare_candidate(vec![addition::server("selected", &["lookup"])], &[&builtin])
                .unwrap(),
        )
        .unwrap();
    let before = active(&runtime);
    let original_name = before.tools.keys().next().unwrap().clone();
    assert_ne!(original_name.as_str(), builtin);
    refresh(&runtime, &["lookup", "new"]).unwrap();
    let after = active(&runtime);
    assert!(after.tools.contains_key(&original_name));
    assert!(after.tools.keys().all(|name| name.as_str() != builtin));
}

#[test]
fn empty_server_refresh_retains_original_configuration_and_authentication() {
    let runtime = standalone();
    install(&runtime, &[]);
    let before = active(&runtime);
    refresh(&runtime, &["first"]).unwrap();
    let after = active(&runtime);
    assert!(Arc::ptr_eq(&before.servers[0], &after.servers[0]));
    let tool = after.tools.values().next().unwrap();
    assert!(tool.binding.live().is_ok());
    assert_eq!(&*after.servers[0].configuration, b"configuration");
    assert_eq!(&*after.servers[0].authentication, b"authentication");
}

#[test]
fn dropped_stale_and_foreign_refresh_preparations_leave_whitelists_unchanged() {
    let runtime = standalone();
    runtime
        .publish(
            runtime
                .prepare_candidate(
                    vec![
                        addition::server("selected", &["old"]),
                        addition::server("other", &["stable"]),
                    ],
                    &[],
                )
                .unwrap(),
        )
        .unwrap();
    let before = active(&runtime);
    let checkpoint = runtime.publication_checkpoint().unwrap();
    let prepared = runtime
        .prepare_tool_refresh(&checkpoint, &before.servers[0], tools(&["dropped"]), &[])
        .unwrap();
    drop(prepared);
    assert!(
        before
            .tools
            .values()
            .all(|tool| tool.binding.live().is_ok())
    );
    let prepared = runtime
        .prepare_tool_refresh(&checkpoint, &before.servers[0], tools(&["foreign"]), &[])
        .unwrap();
    assert!(matches!(
        runtime.commit_tool_refresh(prepared, &mut lane(&before.servers[1])),
        Err(NativeMcpRuntimeError::Invalid)
    ));
    let stale = runtime
        .prepare_tool_refresh(&checkpoint, &before.servers[0], tools(&["stale"]), &[])
        .unwrap();
    refresh(&runtime, &["winner"]).unwrap();
    let winner = active(&runtime);
    assert!(matches!(
        runtime.commit_tool_refresh(stale, &mut lane(&before.servers[0])),
        Err(NativeMcpRuntimeError::Unavailable)
    ));
    assert!(Arc::ptr_eq(&winner, &active(&runtime)));
    assert!(
        winner
            .tools
            .values()
            .all(|tool| tool.binding.live().is_ok())
    );
}

#[test]
fn retained_bindings_remain_charged_after_old_view_and_peer_cleanup_disappear() {
    let mut runtime = standalone();
    install(&runtime, &["old"]);
    let before = active(&runtime);
    let binding = before.tools.values().next().unwrap().binding.clone();
    refresh(&runtime, &["new"]).unwrap();
    drop(before);
    block_on(runtime.drain_retired(
        Instant::now() + Duration::from_secs(1),
        CancellationToken::new(),
    ))
    .unwrap();
    let current = active(&runtime);
    let retained =
        super::super::refresh::retained_catalog_charge(&mut runtime.state.lock().unwrap()).unwrap();
    assert!(retained > 0);
    runtime.limits.max_retained_bytes = current.retained_bytes + retained;
    assert_eq!(
        refresh(&runtime, &["blocked"]),
        Err(NativeMcpRuntimeError::Limit)
    );
    assert!(
        current
            .tools
            .values()
            .all(|tool| tool.binding.live().is_ok())
    );
    drop(binding);
    assert_eq!(
        super::super::refresh::retained_catalog_charge(&mut runtime.state.lock().unwrap()).unwrap(),
        0
    );
}

#[test]
fn refresh_flattens_views_without_reopening_deferred_startup() {
    let runtime = standalone();
    install(&runtime, &["old"]);
    let addition = runtime
        .prepare_addition(
            vec![addition::server("optional", &["extra"])],
            &[],
            &runtime.publication_checkpoint().unwrap(),
        )
        .unwrap();
    runtime.publish_addition(addition).unwrap();
    refresh(&runtime, &["new"]).unwrap();
    let view = active(&runtime);
    assert!(view.previous.is_none());
    assert!(view.deferred_sealed);
    assert_eq!(view.descriptors.len(), 2);
    assert!(matches!(
        runtime.prepare_addition(
            vec![addition::server("again", &[])],
            &[],
            &runtime.publication_checkpoint().unwrap()
        ),
        Err(NativeMcpRuntimeError::Limit)
    ));
}

#[test]
fn old_zero_tool_deferred_view_remains_charged_until_its_owner_releases() {
    let runtime = standalone();
    install(&runtime, &[]);
    let required_view = active(&runtime);
    let addition = runtime
        .prepare_addition(
            vec![addition::server("optional", &["extra"])],
            &[],
            &runtime.publication_checkpoint().unwrap(),
        )
        .unwrap();
    runtime.publish_addition(addition).unwrap();
    refresh(&runtime, &["first"]).unwrap();
    assert!(
        super::super::refresh::retained_catalog_charge(&mut runtime.state.lock().unwrap()).unwrap()
            > 0
    );
    drop(required_view);
    assert_eq!(
        super::super::refresh::retained_catalog_charge(&mut runtime.state.lock().unwrap()).unwrap(),
        0
    );
}

#[test]
fn revoked_selected_authority_rejects_commit_before_retirement() {
    let runtime = standalone();
    let guard = CancellationToken::new();
    let mut server = addition::server("selected", &["old"]);
    server.authority_cancellations = Arc::from([guard.clone()]);
    runtime
        .publish(runtime.prepare_candidate(vec![server], &[]).unwrap())
        .unwrap();
    let before = active(&runtime);
    let mut lane = lane(&before.servers[0]);
    let prepared = runtime
        .prepare_tool_refresh(
            &runtime.publication_checkpoint().unwrap(),
            &before.servers[0],
            tools(&["new"]),
            &[],
        )
        .unwrap();
    guard.cancel();
    assert!(matches!(
        runtime.commit_tool_refresh(prepared, &mut lane),
        Err(NativeMcpRuntimeError::Cancelled)
    ));
    assert!(Arc::ptr_eq(&before, &active(&runtime)));
    assert!(before.check().is_ok());
    assert!(runtime.state.lock().unwrap().retired_catalogs.is_empty());
}
