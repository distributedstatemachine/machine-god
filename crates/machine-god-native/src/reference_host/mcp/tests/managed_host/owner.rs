use super::*;
use crate::NativeManagedAgentsError;
use crate::managed::store::{JournalLimits, ManagedJournal};
use futures_util::future::poll_fn;
use std::{os::unix::fs::DirBuilderExt, pin::Pin, task::Poll};
mod interactive;
mod staged;

fn directory(path: &std::path::Path) -> rustix::fd::OwnedFd {
    rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .unwrap()
}
fn journal_path(fixture: &Fixture) -> std::path::PathBuf {
    let path = fixture.state.join("managed-journal");
    fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
    path
}

#[test]
fn opening_is_inert_and_journal_failure_preserves_original_host_assembly() {
    let mut fixture = Fixture::with_options("ask", true, options);
    let path = journal_path(&fixture);
    let host = fixture.host.as_mut().unwrap();
    let preferences = host.loaded_config().config().model_preferences();
    drop(host.open_managed_agents(
        directory(&path),
        preferences.clone(),
        NativeSessionOrigin::Cli,
    ));
    assert!(host.managed.is_some());
    assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
    run(async {
        let held = ManagedJournal::open(
            directory(&path),
            host.control_workers().unwrap(),
            JournalLimits::default(),
        )
        .await
        .unwrap();
        assert!(matches!(
            host.open_managed_agents(
                directory(&path),
                preferences.clone(),
                NativeSessionOrigin::Cli
            )
            .await,
            Err(NativeManagedAgentsError::Persistence)
        ));
        assert!(host.managed.is_some());
        drop(held);
        let mut agents = host
            .open_managed_agents(
                directory(&path),
                preferences.clone(),
                NativeSessionOrigin::Cli,
            )
            .await
            .unwrap();
        assert!(host.managed.is_none());
        assert!(agents.agents().is_empty());
        assert!(matches!(
            host.open_managed_agents(directory(&path), preferences, NativeSessionOrigin::Cli)
                .await,
            Err(NativeManagedAgentsError::Configuration)
        ));
        poll_fn(|cx| agents.poll_shutdown(cx, 2)).await.unwrap();
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn actual_foreground_model_call_reaches_shared_manager_and_retains_idle_child() {
    let mut fixture = Fixture::with_options("auto", true, options);
    let path = journal_path(&fixture);
    fixture.transport.responses.lock().unwrap().extend([
        call(
            "spawn",
            "subagent",
            &serde_json::json!({"command":{"create":{"name":"worker","mode":"persistent"}}}),
        ),
        answer(),
    ]);
    run(async {
        let host = fixture.host.as_mut().unwrap();
        let preferences = host.loaded_config().config().model_preferences();
        let workspace = host
            .workspace_binding
            .as_ref()
            .unwrap()
            .authority
            .snapshot()
            .unwrap();
        let policy = crate::reference_host::configured_permission_policy(
            host.loaded_config.config(),
            &host.workspace_root,
        )
        .unwrap();
        let mut agents = host
            .open_managed_agents(
                directory(&path),
                preferences.clone(),
                NativeSessionOrigin::Cli,
            )
            .await
            .unwrap();
        let conversation = NativeConversation::create(
            host.session_lifecycle(),
            NativeSessionMetadata::new(&fixture.workspace, 1, NativeSessionOrigin::Cli).unwrap(),
        )
        .await
        .unwrap();
        let reservation = agents.reserve_foreground().unwrap();
        poll_fn(|cx| {
            let progress = agents.poll_progress(cx, 1);
            assert!(!matches!(progress, Poll::Ready(Err(_))));
            agents.poll_foreground_reservation(&reservation, cx)
        })
        .await
        .unwrap();
        let prepared = agents
            .prepare_foreground(conversation, workspace, policy, preferences)
            .await
            .unwrap();
        let selected = agents
            .enroll_foreground(Box::new(prepared), &reservation)
            .unwrap();
        let runtime = agents.foreground_runtime(&selected).unwrap().clone();
        runtime.enqueue("create a worker".into()).unwrap();
        let mut turn = runtime.start_next(2).await.unwrap().unwrap();
        let mut events = Vec::new();
        poll_fn(|cx| {
            use futures_core::Stream;
            let event = Pin::new(&mut turn).poll_next(cx);
            let progress = agents.poll_progress(cx, 3);
            assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
            match event {
                Poll::Ready(Some(event)) => {
                    events.push(event.unwrap().payload);
                    cx.waker().wake_by_ref();
                }
                Poll::Ready(None) => return Poll::Ready(()),
                Poll::Pending => {}
            }
            if progress.is_ready() {
                cx.waker().wake_by_ref();
            }
            Poll::Pending
        })
        .await;
        drop(turn);
        assert!(events.iter().any(|event| matches!(event, TurnEvent::ToolFinished { call_id, output } if call_id.as_str() == "spawn" && !output.is_error)), "{events:?}");
        let children = agents.agents();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "worker");
        assert_eq!(children[0].state, machine_god_core::ManagedAgentState::Idle);
        assert!(agents.selected_runtime(&children[0].selection).is_some());
        assert!(agents.retire_foreground(&selected));
        assert!(agents.foreground_runtime(&selected).is_none());
        assert_eq!(agents.agents().len(), 1);
        poll_fn(|cx| agents.poll_shutdown(cx, 4)).await.unwrap();
    });
    assert_eq!(fixture.transport.requests.lock().unwrap().len(), 2);
}

#[test]
fn parent_only_ephemeral_selection_is_not_inherited_by_children_or_sibling_cancellation() {
    let fixture = Fixture::with_options("ask", true, |selected, directory, clock| {
        let mut selected = options(selected, directory, clock.clone());
        let mcp = selected
            .mcp_runtime
            .take()
            .unwrap()
            .with_ephemeral_startup(super::super::ephemeral::startup(clock))
            .unwrap();
        selected.with_mcp_runtime(mcp)
    });
    let host = fixture.host();
    assert!(host.mcp_runtime().is_none());
    assert!(host.mcp_ephemeral_owner().is_none());
    let parent_seed = host.managed.as_ref().unwrap().parent_mcp.as_ref().unwrap();
    let workers = host.control_workers().unwrap();
    let permissions = host.services.permission_preparation.as_ref().unwrap();
    let a = parent_seed
        .compose(&workers, host.reserved_tool_names(), permissions)
        .unwrap();
    let b = parent_seed
        .compose(&workers, host.reserved_tool_names(), permissions)
        .unwrap();
    let child = host
        .services
        .managed_mcp_seed
        .as_ref()
        .unwrap()
        .compose(&workers, host.reserved_tool_names(), permissions)
        .unwrap();
    assert!(child.ephemeral.is_none());
    assert!(!Arc::ptr_eq(&a.contexts, &b.contexts));
    assert!(!Arc::ptr_eq(&a.runtime, &b.runtime));
    run(async {
        let deadline = Instant::now() + Duration::from_secs(5);
        for parent in [&a, &b] {
            parent
                .ephemeral
                .as_ref()
                .unwrap()
                .replace(
                    crate::mcp::ephemeral::NativeMcpEphemeralConfiguration::decode(None).unwrap(),
                    CancellationToken::new(),
                    deadline,
                )
                .await
                .unwrap();
        }
        a.ephemeral
            .as_ref()
            .unwrap()
            .settle(CancellationToken::new(), deadline)
            .await
            .unwrap();
        b.ephemeral.as_ref().unwrap().ready().unwrap();
        assert!(child.runtime.publication_checkpoint().is_ok());
        b.ephemeral
            .as_ref()
            .unwrap()
            .settle(CancellationToken::new(), deadline)
            .await
            .unwrap();
        child.runtime.close();
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}
