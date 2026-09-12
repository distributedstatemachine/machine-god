use super::*;
use crate::mcp::control::{McpFeatureReply, tests::request};
use std::{future::Future, task::Context};

mod continuation;

fn install(
    runtime: &NativeMcpRuntime,
    writes: Arc<Mutex<Vec<u8>>>,
    revoke: Option<CancellationToken>,
) {
    let source = writes.clone();
    let mut peer = script::ScriptPeer::new(writes).with_response(move |id| {
        let wire = source.lock().unwrap();
        let last = wire.split(|byte| *byte == b'\n').rfind(|line| !line.is_empty()).unwrap();
        let sent: Value = machine_god_core::json::from_slice(last).unwrap();
        let body = match sent["method"].as_str().unwrap() {
            "resources/list" => r#""resources":[{"uri":"test://fixed","name":"fixed"}]"#.to_owned(),
            "resources/templates/list" => r#""resourceTemplates":[{"uriTemplate":"test:///{id}","name":"dynamic"}]"#.to_owned(),
            "prompts/list" => r#""prompts":[{"name":"review","arguments":[{"name":"topic","required":true}]}]"#.to_owned(),
            "resources/read" => format!(r#""contents":[{{"uri":{},"text":"original"}}],"opaque":9007199254740993.00001"#, sent["params"]["uri"]),
            "prompts/get" => r#""messages":[{"role":"assistant","content":{"type":"text","text":"original"}}],"opaque":1e-99999"#.to_owned(),
            "completion/complete" => r#""completion":{"values":["one","one"],"total":2,"hasMore":false}"#.to_owned(),
            _ => panic!("unexpected feature method"),
        };
        drop(wire);
        if let Some(revoke) = &revoke { revoke.cancel(); }
        format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"resultType":"complete",{body}}}}}"#).into_bytes().into()
    });
    peer.tools = false;
    let candidate = runtime
        .prepare_candidate(
            vec![NativeMcpServerCandidate {
                server: Arc::from("fixture"),
                configuration: Arc::from(&b"configuration"[..]),
                authentication: Arc::from(&b"authentication"[..]),
                catalogs: vec![],
                refresh: None,
                catalog_epoch: runtime.clock.now(),
                peer: NativeMcpOwnedPeer::Script(peer),
                operation_timeout: Duration::from_secs(120),
                authority_cancellations: Arc::from([]),
            }],
            &[],
        )
        .unwrap();
    runtime.publish(candidate).unwrap();
}

fn sent(writes: &Mutex<Vec<u8>>) -> Vec<Value> {
    writes
        .lock()
        .unwrap()
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| machine_god_core::json::from_slice(line).unwrap())
        .collect()
}

#[test]
fn human_construction_and_unpolled_exchange_are_inert_and_weak() {
    let runtime = Arc::new(standalone());
    let writes = Arc::<Mutex<Vec<u8>>>::default();
    install(&runtime, writes.clone(), None);
    let owner = runtime.human_command();
    let query = request("resource list fixture");
    drop(owner.feature(&query, CancellationToken::new()));
    assert!(sent(&writes).is_empty());
    assert_eq!(runtime.feature_operations.load(Ordering::Acquire), 0);
    assert!(matches!(
        futures_executor::block_on(owner.feature(&query, CancellationToken::new()))
            .unwrap()
            .reply(),
        McpFeatureReply::Catalog(_)
    ));
    let weak = Arc::downgrade(&runtime);
    drop(runtime);
    assert!(weak.upgrade().is_none());
    assert!(futures_executor::block_on(owner.feature(&query, CancellationToken::new())).is_err());
}

#[test]
fn all_seven_human_actions_use_lazy_exact_catalogs_and_original_results() {
    let runtime = Arc::new(standalone());
    let writes = Arc::<Mutex<Vec<u8>>>::default();
    install(&runtime, writes.clone(), None);
    let owner = runtime.human_command();
    for command in [
        "resource list fixture",
        "resource templates fixture",
        "resource read fixture test://fixed",
        "prompt list fixture",
        r#"prompt get fixture review {"topic":"rust"}"#,
        "prompt complete fixture review topic ru",
        "resource complete fixture test:///{id} id ru",
        "resource read fixture test:///book",
    ] {
        let reply =
            futures_executor::block_on(owner.feature(&request(command), CancellationToken::new()))
                .unwrap();
        if let McpFeatureReply::Response(response) = reply.reply() {
            if command.starts_with("resource read") {
                assert!(response.raw_json().get().contains("9007199254740993.00001"));
            }
            if command.starts_with("prompt get") {
                assert!(response.raw_json().get().contains("1e-99999"));
            }
        }
    }
    let requests = sent(&writes);
    let methods: Vec<_> = requests
        .iter()
        .map(|request| request["method"].as_str().unwrap())
        .collect();
    assert_eq!(
        methods,
        [
            "resources/list",
            "resources/templates/list",
            "resources/list",
            "resources/read",
            "prompts/list",
            "prompts/list",
            "prompts/get",
            "prompts/list",
            "completion/complete",
            "resources/templates/list",
            "completion/complete",
            "resources/list",
            "resources/templates/list",
            "resources/read"
        ]
    );
    for (index, value) in requests.iter().enumerate() {
        assert_eq!(value["id"].as_u64(), Some(index as u64 + 1));
    }
    assert_eq!(runtime.feature_operations.load(Ordering::Acquire), 0);
}

#[test]
fn cancellation_after_discovery_never_writes_the_requested_read() {
    let runtime = Arc::new(standalone());
    let writes = Arc::<Mutex<Vec<u8>>>::default();
    let cancellation = CancellationToken::new();
    install(&runtime, writes.clone(), Some(cancellation.clone()));
    assert!(
        futures_executor::block_on(
            runtime
                .human_command()
                .feature(&request("resource read fixture test://fixed"), cancellation)
        )
        .is_err()
    );
    assert_eq!(sent(&writes).len(), 1);
    assert_eq!(runtime.feature_operations.load(Ordering::Acquire), 0);
}

#[test]
fn closed_or_unknown_human_commands_never_fall_back_to_another_server() {
    let runtime = Arc::new(standalone());
    let writes = Arc::<Mutex<Vec<u8>>>::default();
    install(&runtime, writes.clone(), None);
    let owner = runtime.human_command();
    assert!(
        futures_executor::block_on(
            owner.feature(&request("resource list Fixture"), CancellationToken::new())
        )
        .is_err()
    );
    owner.close();
    assert!(
        futures_executor::block_on(
            owner.feature(&request("resource list fixture"), CancellationToken::new())
        )
        .is_err()
    );
    assert!(sent(&writes).is_empty());
}

#[test]
fn feature_queue_is_bounded_and_command_close_releases_all_slots() {
    let runtime = Arc::new(standalone());
    let writes = Arc::<Mutex<Vec<u8>>>::default();
    install(&runtime, writes.clone(), None);
    let publication = runtime.state.lock().unwrap().active.clone().unwrap();
    let _locked = futures_executor::block_on(publication.servers[0].peer.lock());
    let owner = runtime.human_command();
    let query = request("resource list fixture");
    let mut first = Box::pin(owner.feature(&query, CancellationToken::new()));
    let mut second = Box::pin(owner.feature(&query, CancellationToken::new()));
    let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
    assert!(first.as_mut().poll(&mut cx).is_pending());
    assert!(second.as_mut().poll(&mut cx).is_pending());
    assert_eq!(runtime.feature_operations.load(Ordering::Acquire), 2);
    assert!(matches!(
        futures_executor::block_on(owner.feature(&query, CancellationToken::new())),
        Err(NativeMcpFeatureError::Runtime(NativeMcpRuntimeError::Limit))
    ));
    owner.close();
    assert!(first.as_mut().poll(&mut cx).is_ready());
    assert!(second.as_mut().poll(&mut cx).is_ready());
    assert_eq!(runtime.feature_operations.load(Ordering::Acquire), 0);
    assert_eq!(publication.servers[0].pending.load(Ordering::Acquire), 0);
    assert!(sent(&writes).is_empty());
}

#[test]
fn publication_retirement_cutoff_rejects_queued_features_before_wakeup() {
    let runtime = Arc::new(standalone());
    let writes = Arc::<Mutex<Vec<u8>>>::default();
    install(&runtime, writes.clone(), None);
    let publication = runtime.state.lock().unwrap().active.clone().unwrap();
    let _locked = futures_executor::block_on(publication.servers[0].peer.lock());
    let owner = runtime.human_command();
    let query = request("resource list fixture");
    let mut operation = Box::pin(owner.feature(&query, CancellationToken::new()));
    let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
    assert!(operation.as_mut().poll(&mut cx).is_pending());
    publication.retired.store(true, Ordering::Release);
    assert!(!publication.servers[0].cancellation.is_cancelled());
    assert!(operation.as_mut().poll(&mut cx).is_ready());
    assert!(sent(&writes).is_empty());
}

#[test]
fn actual_model_turn_is_required_and_cannot_rebind_after_reload() {
    use machine_god_testkit::{
        InMemorySessionStore, ScriptedModelProvider, ScriptedPermissionHandler,
    };
    let runtime = Arc::new(standalone());
    let writes = Arc::<Mutex<Vec<u8>>>::default();
    install(&runtime, writes.clone(), None);
    let engine = Engine::builder()
        .session_store(InMemorySessionStore::default())
        .provider(ScriptedModelProvider::new("test", []))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = engine
        .create_session(
            SessionId::new("features").unwrap(),
            SessionIncarnationId::new("exact").unwrap(),
        )
        .unwrap();
    let owner = runtime.contexts.register(&session).unwrap();
    let turn = futures_executor::block_on(session.prompt("features")).unwrap();
    let registration = owner.begin(&session, &turn).unwrap();
    let context = ToolContext {
        session_id: session.id(),
        session_incarnation_id: session.incarnation_id(),
        turn_id: turn.id().clone(),
        call_id: ToolCallId::new("features").unwrap(),
    };
    let query = request("resource list fixture");
    assert!(
        futures_executor::block_on(standalone().feature_for_turn(
            context.clone(),
            &query,
            CancellationToken::new()
        ))
        .is_err()
    );
    futures_executor::block_on(runtime.feature_for_turn(
        context.clone(),
        &query,
        CancellationToken::new(),
    ))
    .unwrap();
    let replacements = Arc::<Mutex<Vec<u8>>>::default();
    install(&runtime, replacements.clone(), None);
    assert!(
        futures_executor::block_on(runtime.feature_for_turn(
            context.clone(),
            &query,
            CancellationToken::new()
        ))
        .is_err()
    );
    assert!(sent(&replacements).is_empty());
    drop(registration);
    assert!(
        futures_executor::block_on(runtime.feature_for_turn(
            context,
            &query,
            CancellationToken::new()
        ))
        .is_err()
    );
    futures_executor::block_on(
        runtime
            .human_command()
            .feature(&query, CancellationToken::new()),
    )
    .unwrap();
    assert_eq!(sent(&replacements).len(), 1);
}

#[test]
fn returned_results_keep_their_bounded_slot_and_original_generation() {
    let runtime = Arc::new(standalone());
    install(&runtime, Arc::default(), None);
    let owner = runtime.human_command();
    let query = request("resource list fixture");
    let first =
        futures_executor::block_on(owner.feature(&query, CancellationToken::new())).unwrap();
    let second =
        futures_executor::block_on(owner.feature(&query, CancellationToken::new())).unwrap();
    first.revalidate().unwrap();
    assert_eq!(runtime.feature_operations.load(Ordering::Acquire), 2);
    assert!(matches!(
        futures_executor::block_on(owner.feature(&query, CancellationToken::new())),
        Err(NativeMcpFeatureError::Runtime(NativeMcpRuntimeError::Limit))
    ));
    install(&runtime, Arc::default(), None);
    assert!(first.revalidate().is_err());
    assert!(second.revalidate().is_err());
    futures_executor::block_on(first.cancelled());
    drop(first);
    assert_eq!(runtime.feature_operations.load(Ordering::Acquire), 1);
    let weak = Arc::downgrade(&runtime);
    drop(runtime);
    assert!(weak.upgrade().is_none());
    assert!(matches!(second.reply(), McpFeatureReply::Catalog(_)));
}

#[test]
fn future_catalog_origin_rejects_before_peer_effects() {
    let runtime = Arc::new(standalone());
    let writes = Arc::<Mutex<Vec<u8>>>::default();
    install(&runtime, writes.clone(), None);
    let future = runtime
        .clock
        .now()
        .checked_add(Duration::from_secs(3600))
        .unwrap();
    {
        let mut state = runtime.state.lock().unwrap();
        let publication = Arc::get_mut(state.active.as_mut().unwrap()).unwrap();
        Arc::get_mut(&mut publication.servers[0])
            .unwrap()
            .catalog_epoch = future;
    }
    assert!(matches!(
        futures_executor::block_on(
            runtime
                .human_command()
                .feature(&request("resource list fixture"), CancellationToken::new())
        ),
        Err(NativeMcpFeatureError::Runtime(
            NativeMcpRuntimeError::Invalid
        ))
    ));
    assert!(sent(&writes).is_empty());
}
