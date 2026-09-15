use super::super::principal::NativePrincipalRegistry;
use super::*;
use crate::file_undo::NativeUndoBudget;
use crate::{
    NativeConfiguredPermissionRules, NativeModelPreferences, NativePermissionPolicySnapshot,
    NativeReasoningEffort, NativeWorkspaceAuthority, PermissionMode, ToolResultArchive,
    mcp::{
        catalog::{McpDescriptorCatalog, McpDescriptorLimits},
        context::NativeMcpContexts,
        pagination::{McpCatalogBuilder, McpCatalogKind, McpCatalogLimits},
        protocol::{ProtocolVersion, RpcId},
        runtime::{
            NativeMcpOwnedPeer, NativeMcpRuntimeClock, NativeMcpRuntimeLimits,
            NativeMcpRuntimeToolCall, NativeMcpScriptPeer, NativeMcpServerCandidate,
            NativeMcpToolExecutionPolicy, NativeMcpToolExecutor,
        },
    },
};
use futures_executor::block_on;
use machine_god_core::{
    Engine, Session, SessionId, SessionIncarnationId, Tool, ToolCallId, ToolError, ToolExecution,
    Turn,
};
use machine_god_testkit::{InMemorySessionStore, ScriptedModelProvider, ScriptedPermissionHandler};
use serde_json::json;
use std::{
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    sync::atomic::AtomicU64,
    time::{Duration, Instant},
};

#[path = "tests/execution.rs"]
mod execution;

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    path: PathBuf,
    workspace: NativeWorkspaceAuthority,
    principals: NativePrincipalRegistry,
    registry: NativePrincipalMcpRegistry,
}
impl Fixture {
    fn new(limit: usize) -> Self {
        let path = std::env::temp_dir().join(format!(
            "machine-god-principal-mcp-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        for name in ["primary", "state", "archive"] {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(path.join(name))
                .unwrap();
        }
        let workspace = NativeWorkspaceAuthority::open_blocking(
            std::fs::File::open(path.join("primary")).unwrap().into(),
            path.join("primary"),
            Some(std::fs::File::open(path.join("state")).unwrap().into()),
            path.join("state"),
            vec![],
            false,
        )
        .unwrap();
        let archive = ToolResultArchive::from_root_descriptor(
            std::fs::File::open(path.join("archive")).unwrap().into(),
        );
        archive.prepare().unwrap();
        let principals =
            NativePrincipalRegistry::new(limit, Arc::new(NativeUndoBudget::default())).unwrap();
        let registry = NativePrincipalMcpRegistry::new(
            limit,
            principals.requester(),
            Arc::new(NativeToolResultArchiveAdapter::new(Arc::new(archive))),
        )
        .unwrap();
        Self {
            path,
            workspace,
            principals,
            registry,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).unwrap();
    }
}
struct Clock;
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep_until(&self, _: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}
struct Executor;
impl NativeMcpToolExecutor for Executor {
    fn execute(
        &self,
        _: NativeMcpRuntimeToolCall,
    ) -> BoxFuture<'_, std::result::Result<ToolExecution, ToolError>> {
        Box::pin(async { panic!("metadata fixture must not execute dynamic calls") })
    }
}
fn runtime() -> (Arc<NativeMcpContexts>, Arc<NativeMcpRuntime>) {
    let contexts = Arc::new(NativeMcpContexts::new());
    let runtime = Arc::new(
        NativeMcpRuntime::new(
            contexts.clone(),
            Arc::new(Clock),
            Arc::new(Executor),
            NativeMcpToolExecutionPolicy::default(),
            NativeMcpRuntimeLimits::default(),
        )
        .unwrap(),
    );
    (contexts, runtime)
}
fn publish(runtime: &NativeMcpRuntime, label: &str, writes: Arc<Mutex<Vec<u8>>>) {
    let response = json!({"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","tools":[{"name":"lookup","description":label,"inputSchema":{"type":"object"}}]}});
    let mut builder = McpCatalogBuilder::new(
        McpCatalogKind::Tools,
        ProtocolVersion::Modern,
        McpCatalogLimits::default(),
    )
    .unwrap();
    builder
        .append_response(
            &serde_json::to_vec(&response).unwrap(),
            &RpcId::Integer(1),
            None,
            0,
        )
        .unwrap();
    let catalog =
        McpDescriptorCatalog::admit(builder.finish().unwrap(), McpDescriptorLimits::default())
            .unwrap();
    let peer=NativeMcpScriptPeer::new(writes).with_response(|id|format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"resultType":"complete","resources":[{{"name":"fixture","uri":"test://fixture"}}]}}}}"#).into_bytes().into());
    let candidate = runtime
        .prepare_candidate(
            vec![NativeMcpServerCandidate {
                server: Arc::from("shared"),
                configuration: Arc::from(label.as_bytes()),
                authentication: Arc::from(&b"test-only"[..]),
                catalogs: vec![catalog],
                refresh: None,
                catalog_epoch: Instant::now(),
                peer: NativeMcpOwnedPeer::Script(peer),
                operation_timeout: Duration::from_secs(10),
                authority_cancellations: Arc::from([]),
            }],
            &[],
        )
        .unwrap();
    runtime.publish(candidate).unwrap();
}
fn engine() -> Engine {
    Engine::builder()
        .provider(ScriptedModelProvider::new("test", []))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .session_store(InMemorySessionStore::default())
        .build()
        .unwrap()
}
fn session(engine: &Engine, id: &str) -> Session {
    engine
        .create_session(
            SessionId::new(id).unwrap(),
            SessionIncarnationId::new(id).unwrap(),
        )
        .unwrap()
}
fn begin(principal: &Arc<NativePrincipal>, turn: &Turn) -> NativePrincipalTurn {
    principal
        .begin_turn(
            turn,
            NativePermissionPolicySnapshot::new(
                PermissionMode::Ask,
                Arc::new(NativeConfiguredPermissionRules::default()),
            ),
            NativeModelPreferences::new(
                "selected",
                NativeReasoningEffort::parse("high").unwrap(),
                false,
            )
            .unwrap(),
            None,
        )
        .unwrap()
}
fn context(session: &Session, turn: &Turn) -> ToolContext {
    ToolContext {
        session_id: session.id(),
        session_incarnation_id: session.incarnation_id(),
        turn_id: turn.id().clone(),
        call_id: ToolCallId::new("repeated").unwrap(),
    }
}
fn snapshot(
    requester: &NativePrincipalMcpRequester,
    session: &Session,
    turn: &Turn,
) -> McpToolCatalogSnapshot {
    block_on(requester.snapshot_for_turn(context(session, turn), CancellationToken::new())).unwrap()
}

#[test]
fn overlapping_names_route_to_independent_runtime_and_reload_close_never_retarget() {
    let f = Fixture::new(2);
    let engine = engine();
    let a = session(&engine, "a");
    let b = session(&engine, "b");
    let pa = f.principals.register(&a, 1, &f.workspace).unwrap();
    let pb = f.principals.register(&b, 1, &f.workspace).unwrap();
    let (ca, ra) = runtime();
    let (cb, rb) = runtime();
    publish(&ra, "alpha", Arc::default());
    publish(&rb, "beta", Arc::default());
    let oa = f.registry.register(&pa, &ra, None).unwrap();
    let ob = f.registry.register(&pb, &rb, None).unwrap();
    let sa = ca.register(&a).unwrap();
    let sb = cb.register(&b).unwrap();
    let ta = block_on(a.prompt("a")).unwrap();
    let tb = block_on(b.prompt("b")).unwrap();
    let _ca = sa.begin(&a, &ta).unwrap();
    let _cb = sb.begin(&b, &tb).unwrap();
    let ga = begin(&pa, &ta);
    let gb = begin(&pb, &tb);
    assert!(oa.begin_turn(&gb).is_err());
    let _ma = oa.begin_turn(&ga).unwrap();
    let _mb = ob.begin_turn(&gb).unwrap();
    let requester = f.registry.requester();
    let old = snapshot(&requester, &a, &ta);
    let sibling = snapshot(&requester, &b, &tb);
    assert_eq!(old.tools()[0].name(), sibling.tools()[0].name());
    assert_eq!(old.tools()[0].description(), "alpha");
    assert_eq!(sibling.tools()[0].description(), "beta");
    let unpolled = requester.snapshot_for_turn(context(&a, &ta), CancellationToken::new());
    publish(&ra, "replacement", Arc::default());
    assert!(block_on(unpolled).is_err());
    assert!(
        block_on(requester.snapshot_for_turn(context(&a, &ta), CancellationToken::new())).is_err()
    );
    assert_eq!(old.tools()[0].description(), "alpha");
    oa.retire();
    assert!(ra.publication_checkpoint().is_err());
    assert_eq!(
        snapshot(&requester, &b, &tb).tools()[0].description(),
        "beta"
    );
    assert!(f.registry.register(&pa, &ra, None).is_err());
    assert!(rb.publication_checkpoint().is_ok());
}

#[test]
fn same_live_runtime_cannot_be_shared_and_failed_registration_does_not_close_it() {
    let f = Fixture::new(3);
    let engine = engine();
    let a = session(&engine, "a");
    let b = session(&engine, "b");
    let pa = f.principals.register(&a, 1, &f.workspace).unwrap();
    let pb = f.principals.register(&b, 1, &f.workspace).unwrap();
    let (_, runtime) = runtime();
    let owner = f.registry.register(&pa, &runtime, None).unwrap();
    assert_eq!(
        f.registry.register(&pb, &runtime, None).unwrap_err(),
        PrincipalMcpError::Duplicate
    );
    assert!(owner.live());
    assert!(runtime.publication_checkpoint().is_ok());
    owner.retire();
    assert!(runtime.publication_checkpoint().is_err());
    assert_eq!(
        f.registry.register(&pb, &runtime, None).unwrap_err(),
        PrincipalMcpError::Unavailable
    );
}

#[test]
fn unpolled_requests_are_inert_stale_guards_reject_and_reverse_edges_are_weak() {
    let f = Fixture::new(1);
    let engine = engine();
    let s = session(&engine, "a");
    let principal = f.principals.register(&s, 1, &f.workspace).unwrap();
    let (contexts, runtime) = runtime();
    let writes = Arc::default();
    publish(&runtime, "alpha", Arc::clone(&writes));
    let owner = f.registry.register(&principal, &runtime, None).unwrap();
    let session_context = contexts.register(&s).unwrap();
    let turn = block_on(s.prompt("a")).unwrap();
    let _native = session_context.begin(&s, &turn).unwrap();
    let principal_turn = begin(&principal, &turn);
    let mcp_turn = owner.begin_turn(&principal_turn).unwrap();
    let requester = f.registry.requester();
    let before = Arc::strong_count(&runtime);
    let pending = requester.snapshot_for_turn(context(&s, &turn), CancellationToken::new());
    assert_eq!(Arc::strong_count(&runtime), before);
    assert!(writes.lock().unwrap().is_empty());
    drop(mcp_turn);
    assert!(block_on(pending).is_err());
    let structural = f.registry.features_tool();
    assert!(
        block_on(structural.execute_for_turn(
            context(&s, &turn),
            json!({}),
            CancellationToken::new()
        ))
        .is_err()
    );
    let weak = Arc::downgrade(&runtime);
    drop(runtime);
    assert!(weak.upgrade().is_none());
    assert!(!owner.live());
    let witness = s.witness();
    drop(turn);
    drop(s);
    drop(engine);
    assert!(!witness.is_live());
}

#[test]
fn registry_drop_closes_owned_runtime_but_retained_selection_only_keeps_cleanup_custody() {
    let f = Fixture::new(1);
    let engine = engine();
    let s = session(&engine, "a");
    let principal = f.principals.register(&s, 1, &f.workspace).unwrap();
    let (_, runtime) = runtime();
    let registry =
        NativePrincipalMcpRegistry::new(1, f.principals.requester(), f.registry.0.archive.clone())
            .unwrap();
    let owner = registry.register(&principal, &runtime, None).unwrap();
    let turn = block_on(s.prompt("a")).unwrap();
    let guard = begin(&principal, &turn);
    let _route = owner.begin_turn(&guard).unwrap();
    let selection = select_captured(registry.requester().capture(&context(&s, &turn))).unwrap();
    let weak = Arc::downgrade(&runtime);
    drop(runtime);
    drop(registry);
    assert!(!selection.route.live());
    assert!(selection.runtime.publication_checkpoint().is_err());
    assert!(weak.upgrade().is_some());
    drop(selection);
    assert!(weak.upgrade().is_none());
}

#[test]
fn unpolled_snapshot_never_retargets_reopened_owner_on_same_actual_turn() {
    let f = Fixture::new(1);
    let engine = engine();
    let s = session(&engine, "a");
    let principal = f.principals.register(&s, 1, &f.workspace).unwrap();
    let (contexts, original) = runtime();
    publish(&original, "original", Arc::default());
    let owner = f.registry.register(&principal, &original, None).unwrap();
    let native_session = contexts.register(&s).unwrap();
    let turn = block_on(s.prompt("a")).unwrap();
    let _native = native_session.begin(&s, &turn).unwrap();
    let guard = begin(&principal, &turn);
    let original_route = owner.begin_turn(&guard).unwrap();
    let requester = f.registry.requester();
    let pending = requester.snapshot_for_turn(context(&s, &turn), CancellationToken::new());
    owner.retire();
    drop(original_route);
    let (replacement_contexts, replacement) = runtime();
    publish(&replacement, "replacement", Arc::default());
    let replacement_session = replacement_contexts.register(&s).unwrap();
    let _replacement_native = replacement_session.begin(&s, &turn).unwrap();
    let replacement_owner = f.registry.register(&principal, &replacement, None).unwrap();
    let _replacement_route = replacement_owner.begin_turn(&guard).unwrap();
    assert!(block_on(pending).is_err());
    assert_eq!(
        snapshot(&requester, &s, &turn).tools()[0].description(),
        "replacement"
    );
}
