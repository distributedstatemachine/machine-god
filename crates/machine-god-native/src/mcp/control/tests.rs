use super::*;
use crate::{
    McpFeatureRequest,
    mcp::{
        commands::McpCommand,
        pagination::{McpCatalogBuilder, McpCatalogKind},
        protocol::{ProtocolVersion, RpcId},
    },
};

pub(crate) fn human(token: CancellationToken) -> McpFeatureControlAuthority {
    human_selected(token, Arc::new(AtomicBool::new(false)))
}
pub(crate) fn human_selected(
    token: CancellationToken,
    retired: Arc<AtomicBool>,
) -> McpFeatureControlAuthority {
    McpFeatureControlAuthority::for_human(
        token,
        CancellationToken::new(),
        CancellationToken::new(),
        retired,
        Arc::from([]),
    )
    .unwrap()
}
pub(crate) fn request(command: &str) -> McpFeatureRequest {
    let McpCommand::Feature(command) = command.parse::<McpCommand>().unwrap() else {
        panic!()
    };
    McpFeatureRequest::try_from(command).unwrap()
}
pub(crate) fn catalogs() -> Vec<McpDescriptorCatalog> {
    [
        (
            McpCatalogKind::Resources,
            "resources",
            r#"[{"uri":"test://fixed","name":"fixed"}]"#,
        ),
        (
            McpCatalogKind::ResourceTemplates,
            "resourceTemplates",
            r#"[{"uriTemplate":"test:///{id}","name":"dynamic"}]"#,
        ),
        (
            McpCatalogKind::Prompts,
            "prompts",
            r#"[{"name":"review","arguments":[{"name":"topic","required":true}]}]"#,
        ),
    ]
    .map(|(kind, field, items)| {
        let raw = format!(
            r#"{{"jsonrpc":"2.0","id":1,"result":{{"resultType":"complete","{field}":{items}}}}}"#
        );
        let mut builder =
            McpCatalogBuilder::new(kind, ProtocolVersion::Modern, McpCatalogLimits::default())
                .unwrap();
        builder
            .append_response(raw.as_bytes(), &RpcId::Integer(1), None, 0)
            .unwrap();
        McpDescriptorCatalog::admit(builder.finish().unwrap(), McpDescriptorLimits::default())
            .unwrap()
    })
    .into()
}

#[test]
fn human_authority_observes_every_selected_signal_and_bounded_guards() {
    for selected in 0..11 {
        let tokens: Vec<_> = (0..11).map(|_| CancellationToken::new()).collect();
        let authority = McpFeatureControlAuthority::for_human(
            tokens[0].clone(),
            tokens[1].clone(),
            tokens[2].clone(),
            Arc::new(AtomicBool::new(false)),
            tokens[3..].into(),
        )
        .unwrap();
        let mut cancelled = authority.cancelled();
        let mut cx = std::task::Context::from_waker(futures_util::task::noop_waker_ref());
        assert!(cancelled.as_mut().poll(&mut cx).is_pending());
        tokens[selected].cancel();
        assert!(!authority.is_live());
        assert!(cancelled.as_mut().poll(&mut cx).is_ready());
    }
    assert!(
        McpFeatureControlAuthority::for_human(
            CancellationToken::new(),
            CancellationToken::new(),
            CancellationToken::new(),
            Arc::new(AtomicBool::new(false)),
            (0..9).map(|_| CancellationToken::new()).collect()
        )
        .is_err()
    );
}

#[test]
fn actual_model_turn_retirement_is_independent_of_supplied_tokens() {
    use machine_god_core::{Engine, SessionId, SessionIncarnationId, ToolCallId, ToolContext};
    use machine_god_testkit::{InMemorySessionStore, ScriptedModelProvider};
    let engine = Engine::builder()
        .session_store(InMemorySessionStore::default())
        .provider(ScriptedModelProvider::new("test", []))
        .permission_handler(machine_god_testkit::ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = engine
        .create_session(
            SessionId::new("features").unwrap(),
            SessionIncarnationId::new("lifetime").unwrap(),
        )
        .unwrap();
    let contexts = super::super::context::NativeMcpContexts::new();
    let owner = contexts.register(&session).unwrap();
    let turn = futures_executor::block_on(session.prompt("request")).unwrap();
    let registration = owner.begin(&session, &turn).unwrap();
    let context = ToolContext {
        session_id: session.id(),
        session_incarnation_id: session.incarnation_id(),
        turn_id: turn.id().clone(),
        call_id: ToolCallId::new("call").unwrap(),
    };
    let authority = McpFeatureControlAuthority::for_model(
        Arc::new(contexts.snapshot_for_tool(&context).unwrap()),
        CancellationToken::new(),
        CancellationToken::new(),
        Arc::new(AtomicBool::new(false)),
        Arc::from([]),
    )
    .unwrap();
    assert!(authority.is_live());
    drop(registration);
    assert!(!authority.is_live());
    futures_executor::block_on(authority.cancelled());
}

#[test]
fn guarded_future_checks_after_inner_poll_and_unpolled_drop_does_nothing() {
    let token = CancellationToken::new();
    let authority = human(token.clone());
    let executed = std::sync::atomic::AtomicBool::new(false);
    drop(guarded(&authority, async {
        executed.store(true, std::sync::atomic::Ordering::Relaxed);
    }));
    assert!(!executed.load(std::sync::atomic::Ordering::Relaxed));
    assert!(
        futures_executor::block_on(guarded(&authority, async {
            token.cancel();
            7
        }))
        .is_err()
    );
}

#[test]
fn retirement_cutoff_precedes_deferred_cancellation_wakeup() {
    let token = CancellationToken::new();
    let retired = Arc::new(AtomicBool::new(false));
    let authority = human_selected(token.clone(), retired.clone());
    let mut cancelled = authority.cancelled();
    let mut cx = std::task::Context::from_waker(futures_util::task::noop_waker_ref());
    assert!(cancelled.as_mut().poll(&mut cx).is_pending());
    retired.store(true, Ordering::Release);
    assert!(!token.is_cancelled());
    assert!(!authority.is_live());
    assert!(cancelled.as_mut().poll(&mut cx).is_ready());
}
