use super::*;
use crate::{NativeManagedInteractiveStartup, NativeReferenceHost, NativeResumeTarget};

pub(super) async fn prepare(
    fixture: &mut Fixture,
) -> (NativeManagedInteractiveStartup, Arc<NativeReferenceHost>) {
    let path = journal_path(fixture);
    let mut host = fixture.host.take().unwrap();
    let preferences = host.loaded_config().config().model_preferences();
    let options =
        NativeInteractiveSessionOptions::new(fixture.workspace.clone(), preferences.clone())
            .unwrap();
    let agents = host
        .open_managed_agents(directory(&path), preferences, NativeSessionOrigin::Cli)
        .await
        .unwrap();
    assert!(
        host.managed_agents_selected(),
        "selection survives outer assembly transfer"
    );
    let host = Arc::new(host);
    let startup = NativeManagedInteractiveStartup::new(host.clone(), options, agents).unwrap();
    (startup, host)
}

#[test]
fn failed_selection_keeps_original_manager_for_explicit_retry_and_transfer() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut startup, host) = prepare(&mut fixture).await;
        let completion = host.terminal_shutdown_completion().unwrap();
        startup
            .request_open(
                NativeInteractiveInitialSession::Resume(NativeResumeTarget::Exact(
                    machine_god_core::SessionId::new("missing-session").unwrap(),
                )),
                1,
            )
            .unwrap();
        assert!(matches!(
            startup.request_open(NativeInteractiveInitialSession::Fresh, 2),
            Err(crate::NativeInteractiveError::Busy)
        ));
        assert!(fixture.transport.requests.lock().unwrap().is_empty());
        assert!(poll_fn(|cx| startup.poll_open(cx, 2)).await.is_err());
        startup
            .request_open(NativeInteractiveInitialSession::Fresh, 3)
            .unwrap();
        let mut owner = poll_fn(|cx| startup.poll_open(cx, 3))
            .await
            .unwrap()
            .unwrap();
        // Dropping the preselection display cannot cancel the transferred owner.
        drop(startup);
        owner.enqueue("actual first input".into()).unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeInteractiveOutcome::Turn(Ok(_))
        ));
        owner.request_shutdown();
        while !owner.is_closed() {
            let _ = outcome(&mut owner).await;
        }
        drop(owner);
        drop(host);
        completion.wait().await;
    });
    assert_eq!(fixture.transport.requests.lock().unwrap().len(), 1);
}

#[test]
fn cancelling_unpolled_selection_closes_without_provider_execution_or_owner_transfer() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut startup, host) = prepare(&mut fixture).await;
        let completion = host.terminal_shutdown_completion().unwrap();
        startup
            .request_open(NativeInteractiveInitialSession::Fresh, 1)
            .unwrap();
        startup.request_shutdown();
        assert!(
            poll_fn(|cx| startup.poll_open(cx, 2))
                .await
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            startup.request_open(NativeInteractiveInitialSession::Fresh, 3),
            Err(crate::NativeInteractiveError::Closed)
        ));
        drop(startup);
        drop(host);
        completion.wait().await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn foreign_host_binding_returns_original_manager_without_closing_either_host() {
    let mut fixture = Fixture::with_options("auto", true, options);
    let mut foreign = Fixture::with_options("auto", true, options);
    run(async {
        let mut host = fixture.host.take().unwrap();
        let preferences = host.loaded_config().config().model_preferences();
        let options =
            NativeInteractiveSessionOptions::new(fixture.workspace.clone(), preferences.clone())
                .unwrap();
        let agents = host
            .open_managed_agents(
                directory(&journal_path(&fixture)),
                preferences,
                NativeSessionOrigin::Cli,
            )
            .await
            .unwrap();
        let host = Arc::new(host);
        let foreign_host = Arc::new(foreign.host.take().unwrap());
        let completion = host.terminal_shutdown_completion().unwrap();
        let foreign_completion = foreign_host.terminal_shutdown_completion().unwrap();
        let (error, agents) =
            NativeManagedInteractiveStartup::new(foreign_host.clone(), options.clone(), agents)
                .unwrap_err();
        assert!(matches!(
            error,
            crate::NativeInteractiveError::Configuration
        ));
        let mut startup =
            NativeManagedInteractiveStartup::new(host.clone(), options, *agents).unwrap();
        startup.request_shutdown();
        assert!(
            poll_fn(|cx| startup.poll_open(cx, 1))
                .await
                .unwrap()
                .is_none()
        );
        let foreign_session = NativeConversation::create(
            foreign_host.session_lifecycle(),
            NativeSessionMetadata::new(&foreign.workspace, 1, NativeSessionOrigin::Cli).unwrap(),
        )
        .await
        .unwrap();
        drop(foreign_session);
        drop(startup);
        drop(host);
        drop(foreign_host);
        completion.wait().await;
        foreign_completion.wait().await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
    assert!(foreign.transport.requests.lock().unwrap().is_empty());
}
