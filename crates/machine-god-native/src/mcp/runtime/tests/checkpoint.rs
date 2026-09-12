use super::*;

#[test]
fn conditional_initial_publication_and_reload_require_the_exact_observation() {
    let runtime = standalone();
    let empty = runtime.publication_checkpoint().unwrap();
    runtime
        .publish_if(
            candidate(&runtime, "server", &["one"], Arc::default()),
            &empty,
        )
        .unwrap();
    let initial = runtime.publication_checkpoint().unwrap();
    runtime
        .publish_if(
            candidate(&runtime, "server", &["two"], Arc::default()),
            &initial,
        )
        .unwrap();
    let active = runtime.state.lock().unwrap().active.clone().unwrap();
    for stale in [&empty, &initial] {
        assert_eq!(
            runtime.publish_if(
                candidate(&runtime, "server", &["three"], Arc::default()),
                stale
            ),
            Err(NativeMcpRuntimeError::Unavailable)
        );
        let state = runtime.state.lock().unwrap();
        assert!(Arc::ptr_eq(state.active.as_ref().unwrap(), &active));
        assert_eq!(state.retired.len(), 1);
        assert!(
            active
                .tools
                .values()
                .all(|tool| tool.binding.live().is_ok())
        );
        assert!(!active.servers[0].cancellation.is_cancelled());
    }
}

#[test]
fn a_foreign_empty_checkpoint_cannot_authorize_initial_publication() {
    let runtime = standalone();
    let foreign = standalone().publication_checkpoint().unwrap();
    assert_eq!(
        runtime.publish_if(
            candidate(&runtime, "server", &["one"], Arc::default()),
            &foreign
        ),
        Err(NativeMcpRuntimeError::Invalid)
    );
    assert!(runtime.state.lock().unwrap().active.is_none());
}

#[test]
fn checkpoints_neither_keep_peers_alive_nor_reopen_a_closed_runtime() {
    let runtime = Arc::new(standalone());
    runtime
        .publish(candidate(&runtime, "server", &["one"], Arc::default()))
        .unwrap();
    let checkpoint = runtime.publication_checkpoint().unwrap();
    let old = Arc::downgrade(runtime.state.lock().unwrap().active.as_ref().unwrap());
    runtime
        .publish(candidate(&runtime, "server", &["two"], Arc::default()))
        .unwrap();
    assert!(old.upgrade().is_none());
    runtime.close();
    assert!(runtime.publication_checkpoint().is_err());
    assert_eq!(
        runtime.publish_if(
            candidate(&runtime, "server", &["three"], Arc::default()),
            &checkpoint
        ),
        Err(NativeMcpRuntimeError::Unavailable)
    );
    let owner = Arc::downgrade(&runtime);
    drop(runtime);
    assert!(owner.upgrade().is_none());
    drop(checkpoint);
}
