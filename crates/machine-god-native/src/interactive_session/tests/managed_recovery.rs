use super::{Fixture, executor};
use crate::{
    NativeInteractiveInitialSession, NativeInteractiveSession, NativeInteractiveSessionOptions,
    NativeManagedInteractiveStartup, NativeReferenceHostManagedOptions, NativeSessionOrigin,
};
use std::{
    future::{Future, poll_fn},
    sync::Arc,
    task::Poll,
    time::Duration,
};

pub(crate) async fn prepared() -> (Fixture, NativeInteractiveSession) {
    let mut fixture = Fixture::with_workspace_options(|options| {
        options.with_managed_agents(NativeReferenceHostManagedOptions::new(Arc::new(
            crate::mcp::clock::TokioMcpClock,
        )))
    });
    let directory = std::fs::File::open(fixture.state_root()).unwrap();
    let host = Arc::get_mut(&mut fixture.host).unwrap();
    let preferences = host.loaded_config().config().model_preferences();
    let agents = host
        .open_workspace_managed_agents(
            directory.into(),
            preferences.clone(),
            NativeSessionOrigin::Cli,
        )
        .await
        .unwrap();
    let options =
        NativeInteractiveSessionOptions::new(fixture.workspace.clone(), preferences).unwrap();
    let mut startup =
        NativeManagedInteractiveStartup::new(fixture.host.clone(), options, agents).unwrap();
    startup
        .request_open(NativeInteractiveInitialSession::Fresh, 100)
        .unwrap();
    let owner = poll_fn(|cx| startup.poll_open(cx, 100))
        .await
        .unwrap()
        .unwrap();
    (fixture, owner)
}

#[test]
fn native_owner_retains_persistent_clear_failure_and_recovers_without_input() {
    executor().block_on(async {
        let (fixture, mut owner) = prepared().await;
        let blocked = fixture.fence_notice_clear(&mut owner).await;
        let providers = fixture.transport.requests().len();
        owner.request_shutdown();
        let delay = tokio::time::sleep(Duration::from_millis(1100));
        tokio::pin!(delay);
        poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 102);
            let _ = owner.take_presentation();
            let _ = owner.take_outcome();
            assert!(!owner.is_closed());
            assert!(owner.managed_progress().unwrap().residents > 0);
            delay.as_mut().poll(cx)
        })
        .await;
        drop(blocked);
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                let _ = owner.poll_progress(cx, 102);
                let _ = owner.take_presentation();
                let _ = owner.take_outcome();
                if owner.is_closed() {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }),
        )
        .await
        .unwrap();
        assert!(owner.shutdown_error().is_none());
        assert_eq!(fixture.transport.requests().len(), providers);
        drop(owner);
        fixture.finish();
    });
}
