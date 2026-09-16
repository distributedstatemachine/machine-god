use super::*;
use crate::{NativeManagedInteractiveStartup, NativeReferenceHost};

async fn prepare(
    fixture: &mut Fixture,
) -> (NativeManagedInteractiveStartup, Arc<NativeReferenceHost>) {
    let path = journal_path(fixture);
    let mut host = fixture.host.take().unwrap();
    let preferences = host.loaded_config().config().model_preferences();
    let agents = host
        .open_managed_agents(
            directory(&path),
            preferences.clone(),
            NativeSessionOrigin::Acp,
        )
        .await
        .unwrap();
    let options = NativeInteractiveSessionOptions::new(fixture.workspace.clone(), preferences)
        .unwrap()
        .with_origin(NativeSessionOrigin::Acp);
    let host = Arc::new(host);
    let startup = NativeManagedInteractiveStartup::new(host.clone(), options, agents).unwrap();
    (startup, host)
}

async fn stage(
    startup: &mut NativeManagedInteractiveStartup,
    configuration: NativeMcpEphemeralConfiguration,
) -> crate::reference_host::NativeManagedStagedParent {
    let reservation = startup.reserve_parent_stage().unwrap();
    poll_fn(|cx| {
        assert!(startup.poll_open(cx, 1).is_pending());
        startup.poll_parent_stage_reservation(&reservation, cx)
    })
    .await
    .unwrap();
    startup
        .start_parent_stage(reservation, configuration, CancellationToken::new())
        .await
        .unwrap()
}

async fn empty(
    startup: &mut NativeManagedInteractiveStartup,
) -> crate::reference_host::NativeManagedStagedParent {
    let candidate = stage(
        startup,
        NativeMcpEphemeralConfiguration::decode(None).unwrap(),
    )
    .await;
    candidate.ready().unwrap();
    candidate
}

#[test]
fn first_staged_open_publishes_one_session_without_a_temporary_parent() {
    let mut fixture = fixture();
    run(async {
        let (mut startup, host) = prepare(&mut fixture).await;
        let completion = host.terminal_shutdown_completion().unwrap();
        let candidate = empty(&mut startup).await;
        assert!(
            host.session_lifecycle()
                .list_sessions()
                .await
                .unwrap()
                .session_ids()
                .is_empty()
        );
        startup
            .request_open_staged(NativeInteractiveInitialSession::Fresh, candidate, 2)
            .unwrap();
        assert!(
            host.session_lifecycle()
                .list_sessions()
                .await
                .unwrap()
                .session_ids()
                .is_empty()
        );
        let mut owner = poll_fn(|cx| startup.poll_open(cx, 2))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            host.session_lifecycle()
                .list_sessions()
                .await
                .unwrap()
                .session_ids(),
            &[owner.runtime().id()]
        );
        assert!(
            !owner
                .acp_mcp_runtime()
                .unwrap()
                .publication_checkpoint()
                .unwrap()
                .is_unpublished()
        );
        drop(startup);
        close_interactive(&mut owner).await;
        drop(owner);
        drop(host);
        completion.wait().await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn staged_resume_failure_settles_before_explicit_retry_with_the_same_manager() {
    let mut fixture = fixture();
    run(async {
        let (mut startup, host) = prepare(&mut fixture).await;
        let completion = host.terminal_shutdown_completion().unwrap();
        let candidate = empty(&mut startup).await;
        startup
            .request_open_staged(
                NativeInteractiveInitialSession::Resume(crate::NativeResumeTarget::Exact(
                    machine_god_core::SessionId::new("missing-staged-startup").unwrap(),
                )),
                candidate,
                2,
            )
            .unwrap();
        assert!(poll_fn(|cx| startup.poll_open(cx, 2)).await.is_err());
        assert!(!startup.is_finished());
        assert!(
            host.session_lifecycle()
                .list_sessions()
                .await
                .unwrap()
                .session_ids()
                .is_empty()
        );
        let retry = empty(&mut startup).await;
        startup
            .request_open_staged(NativeInteractiveInitialSession::Fresh, retry, 3)
            .unwrap();
        let mut owner = poll_fn(|cx| startup.poll_open(cx, 3))
            .await
            .unwrap()
            .unwrap();
        drop(startup);
        close_interactive(&mut owner).await;
        drop(owner);
        drop(host);
        completion.wait().await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn shutdown_of_unpolled_staged_open_settles_without_creating_a_transcript() {
    let mut fixture = fixture();
    run(async {
        let (mut startup, host) = prepare(&mut fixture).await;
        let completion = host.terminal_shutdown_completion().unwrap();
        let candidate = empty(&mut startup).await;
        startup
            .request_open_staged(NativeInteractiveInitialSession::Fresh, candidate, 2)
            .unwrap();
        startup.request_shutdown();
        assert!(
            poll_fn(|cx| startup.poll_open(cx, 3))
                .await
                .unwrap()
                .is_none()
        );
        assert!(startup.is_finished());
        assert!(
            host.session_lifecycle()
                .list_sessions()
                .await
                .unwrap()
                .session_ids()
                .is_empty()
        );
        drop(startup);
        drop(host);
        completion.wait().await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn rejected_ready_check_returns_original_custody_and_charges_until_released() {
    let mut fixture = fixture();
    run(async {
        let (mut startup, host) = prepare(&mut fixture).await;
        let completion = host.terminal_shutdown_completion().unwrap();
        let candidate = stage(
            &mut startup,
            NativeMcpEphemeralConfiguration::decode(Some(
                br#"[{"name":"missing","command":"/bin/sh","args":[],"env":[]}]"#,
            ))
            .unwrap(),
        )
        .await;
        assert!(candidate.ready().is_err());
        let mut failure = startup
            .request_open_staged(NativeInteractiveInitialSession::Fresh, candidate, 2)
            .unwrap_err();
        startup.request_shutdown();
        let mut cx = std::task::Context::from_waker(futures_util::task::noop_waker_ref());
        assert!(startup.poll_open(&mut cx, 3).is_pending());
        failure.settle().await.unwrap();
        assert!(startup.poll_open(&mut cx, 3).is_pending());
        drop(failure);
        assert!(
            poll_fn(|cx| startup.poll_open(cx, 3))
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            host.session_lifecycle()
                .list_sessions()
                .await
                .unwrap()
                .session_ids()
                .is_empty()
        );
        drop(startup);
        drop(host);
        completion.wait().await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}
