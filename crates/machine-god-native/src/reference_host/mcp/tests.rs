use super::*;
use crate::reference_host::{
    NativeReferenceHostConversationOptions, NativeReferenceHostPermissionOptions,
    NativeReferenceHostTerminalOptions, PreparedCompositionOptions, validate_prepared_selections,
};
use crate::*;
use futures_util::StreamExt;
use machine_god_core::{BoxFuture, ToolCallId, ToolContext, TurnEvent};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

mod fixture;
#[cfg(feature = "mcp-http")]
mod http;
use fixture::*;

#[derive(Default)]
struct CountingPresenter(AtomicUsize);
impl crate::mcp::interaction::McpElicitationPresenter for CountingPresenter {
    fn present(
        &self,
        _: crate::mcp::interaction::McpElicitationPromptRequest,
        _: machine_god_core::CancellationToken,
    ) -> BoxFuture<
        '_,
        Result<
            crate::mcp::interaction::McpElicitationAnswer,
            crate::mcp::interaction::McpElicitationPromptError,
        >,
    > {
        self.0.fetch_add(1, Ordering::Relaxed);
        Box::pin(std::future::pending())
    }
}

#[test]
fn form_responder_selection_retains_the_actual_endpoint_without_prompting() {
    let presenter = Arc::new(CountingPresenter::default());
    let erased: Arc<dyn crate::mcp::interaction::McpElicitationPresenter> = presenter.clone();
    let options = NativeReferenceHostMcpOptions::new(
        Arc::new(NativeMcpContexts::new()),
        Arc::new(Clock::default()),
    )
    .with_form_responder(erased.clone());
    assert!(Arc::ptr_eq(
        options.form_responder.as_ref().unwrap(),
        &erased
    ));
    assert!(Arc::ptr_eq(
        options.clone().form_responder.as_ref().unwrap(),
        &erased
    ));
    assert_eq!(presenter.0.load(Ordering::Relaxed), 0);
}

#[test]
fn runtime_options_are_inert_and_require_exact_context_permission_and_terminal_selection() {
    let contexts = Arc::new(NativeMcpContexts::new());
    let clock = Arc::new(Clock::default());
    let options = NativeReferenceHostMcpOptions::new(contexts.clone(), clock.clone());
    let config = LoadedNativeConfig::from_file(NativeConfig::default());
    let make = || {
        NativeReferenceHostConversationOptions::new(Arc::new(FileUndoTracker::new()))
            .with_mcp_runtime(options.clone())
    };
    let valid = || {
        make()
            .with_terminal(terminal())
            .with_permissions(permission(clock.clone()))
    };
    for selected in [
        make(),
        make().with_terminal(terminal()),
        make().with_permissions(permission(clock.clone())),
        valid().with_mcp_contexts(Arc::new(NativeMcpContexts::new())),
    ] {
        assert_eq!(
            validate_prepared_selections(&config, &selected.into())
                .unwrap_err()
                .kind(),
            NativeReferenceHostBuildErrorKind::McpConfig
        );
    }
    let selected: PreparedCompositionOptions = valid().with_mcp_contexts(contexts.clone()).into();
    validate_prepared_selections(&config, &selected).unwrap();
    assert!(Arc::ptr_eq(
        &selected.mcp_runtime.unwrap().contexts,
        &contexts
    ));
    assert_eq!(clock.0.load(Ordering::Relaxed), 0);
    assert_eq!(
        format!("{options:?}"),
        "NativeReferenceHostMcpOptions { <redacted> }"
    );
}

#[test]
fn concrete_executor_composition_does_not_prepare_archive_or_read_clock() {
    let directory = Directory::new();
    let archive = Arc::new(ToolResultArchive::from_root_descriptor(
        rustix::fs::open(
            &directory.0,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY,
            rustix::fs::Mode::empty(),
        )
        .unwrap(),
    ));
    let workers = NativeOwnedWorkerScope::new();
    let clock = Arc::new(Clock::default());
    let options =
        NativeReferenceHostMcpOptions::new(Arc::new(NativeMcpContexts::new()), clock.clone());
    let composition = options
        .compose(Arc::new(
            NativeToolResultArchiveAdapter::new(archive).with_worker_scope(workers.clone()),
        ))
        .unwrap();
    assert_eq!(clock.0.load(Ordering::Relaxed), 0);
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
    composition.runtime.close();
    workers.close();
    assert!(workers.completion().is_complete());
}

#[test]
fn actual_host_retains_exact_contexts_reserved_names_and_inert_conversation_enrollment() {
    for enabled in [false, true] {
        let fixture = Fixture::new("ask", enabled);
        let host = fixture.host();
        assert_eq!(host.mcp_runtime().is_some(), enabled);
        assert_eq!(host.reserved_tool_names().len(), 26);
        assert_eq!(
            host.reserved_tool_names(),
            host.engine()
                .tool_specs()
                .iter()
                .map(|spec| spec.name.clone())
                .collect::<Vec<_>>()
        );
        assert!(Arc::ptr_eq(
            &host.mcp_contexts().unwrap(),
            &fixture.contexts
        ));
        assert!(fixture.transport.requests.lock().unwrap().is_empty());
        assert_eq!(fixture.clock.0.load(Ordering::Relaxed), 0);
        run(async {
            let conversation = fixture.conversation().await;
            let turn = conversation.prompt("question".into(), 10).await.unwrap();
            let context = ToolContext {
                session_id: conversation.id(),
                session_incarnation_id: conversation.incarnation_id(),
                turn_id: turn.handle().id().clone(),
                call_id: ToolCallId::new("unexecuted").unwrap(),
            };
            let snapshot = fixture.contexts.snapshot_for_tool(&context).unwrap();
            if let Some(runtime) = host.mcp_runtime() {
                assert!(
                    runtime
                        .snapshot_for_turn(context.clone(), CancellationToken::new())
                        .await
                        .is_ok()
                );
                assert!(runtime.snapshot(CancellationToken::new()).await.is_err());
                let foreign = ToolContext {
                    session_incarnation_id: machine_god_core::SessionIncarnationId::new("foreign")
                        .unwrap(),
                    ..context.clone()
                };
                assert!(
                    runtime
                        .snapshot_for_turn(foreign, CancellationToken::new())
                        .await
                        .is_err()
                );
            }
            drop(turn);
            assert!(!snapshot.is_live());
            assert!(fixture.transport.requests.lock().unwrap().is_empty());
        });
    }
}

#[test]
fn into_engine_keeps_actual_mcp_owner_but_retained_requesters_cannot_keep_it_live() {
    let mut fixture = Fixture::new("ask", true);
    let host = fixture.host.take().unwrap();
    let completion = host.terminal_shutdown_completion().unwrap();
    let runtime = host.mcp_runtime().unwrap();
    let requester = host.engine().requester();
    let engine = host.into_engine();
    runtime
        .publish(runtime.prepare_candidate(vec![], &[]).unwrap())
        .unwrap();
    drop(engine);
    assert!(
        runtime
            .publish(runtime.prepare_candidate(vec![], &[]).unwrap())
            .is_err()
    );
    completion.wait_on_worker().unwrap();
    assert!(completion.is_complete());
    drop(requester);
}

#[test]
fn two_actual_hosts_never_route_the_other_conversations_turn() {
    let first = Fixture::new("ask", true);
    let second = Fixture::new("ask", true);
    run(async {
        let first_conversation = first.conversation().await;
        let second_conversation = second.conversation().await;
        let first_turn = first_conversation.prompt("first".into(), 10).await.unwrap();
        let second_turn = second_conversation
            .prompt("second".into(), 10)
            .await
            .unwrap();
        let context = ToolContext {
            session_id: second_conversation.id(),
            session_incarnation_id: second_conversation.incarnation_id(),
            turn_id: second_turn.handle().id().clone(),
            call_id: ToolCallId::new("lookup").unwrap(),
        };
        assert!(second.contexts.snapshot_for_tool(&context).is_ok());
        assert!(first.contexts.snapshot_for_tool(&context).is_err());
        assert!(
            first
                .host()
                .mcp_runtime()
                .unwrap()
                .snapshot_for_turn(context.clone(), CancellationToken::new())
                .await
                .is_err()
        );
        drop(second_turn);
        assert!(second.contexts.snapshot_for_tool(&context).is_err());
        drop(first_turn);
        assert!(first.transport.requests.lock().unwrap().is_empty());
        assert!(second.transport.requests.lock().unwrap().is_empty());
    });
}

#[test]
fn actual_builtin_permission_and_unknown_tool_fail_closed_with_mcp_selected() {
    let fixture = Fixture::new("ask", true);
    fixture.transport.responses.lock().unwrap().extend([
        call(
            "write",
            WRITE_FILE_TOOL_NAME,
            &serde_json::json!({"path":"created.txt","content":"native"}),
        ),
        call("unknown", "unregistered_mcp_name", &serde_json::json!({})),
        answer(),
    ]);
    run(async {
        let conversation = fixture.conversation().await;
        let runtime = NativeConversationRuntime::new(
            conversation,
            fixture.host().loaded_config().config().model_preferences(),
            None,
        )
        .unwrap();
        let events = collect(&runtime).await;
        assert!(events.iter().any(|event| matches!(event, TurnEvent::ToolFinished { call_id, output } if call_id.as_str() == "write" && !output.is_error)));
        assert!(events.iter().any(
            |event| matches!(event, TurnEvent::Failed { code, .. } if code == "unknown_tool")
        ));
        assert_eq!(
            fs::read_to_string(fixture.workspace.join("created.txt")).unwrap(),
            "native"
        );
        assert_eq!(fixture.prompt.calls.load(Ordering::Relaxed), 1);
        assert_eq!(fixture.transport.reviews.load(Ordering::Relaxed), 0);
    });
}
