use super::*;
use crate::{
    NativeEnvironment, NativeRootSelection,
    mcp::{lifetime::McpPeerLifetime, network::McpResolverConfig, peer::McpStdioLaunchFactory},
};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt},
};

#[test]
fn captured_startup_reuses_retained_roots_environment_and_exact_clock() {
    let directory = Directory::new();
    let workspace = directory.0.join("workspace");
    let state = directory.0.join("state");
    fs::create_dir(&workspace).unwrap();
    fs::DirBuilder::new().mode(0o700).create(&state).unwrap();
    let environment = NativeEnvironment::new(None, Some(state.into_os_string()), None);
    let roots = PreparedNativeRoots::prepare(
        NativeRootSelection::from_environment(&environment, &workspace).unwrap(),
    )
    .unwrap();
    let original = fs::metadata(&workspace).unwrap();
    fs::rename(&workspace, directory.0.join("retained-workspace")).unwrap();
    let entries = vec![("MCP_CAPTURE_TEST".into(), "exact-selected-value".into())];
    let terminal = NativeReferenceHostTerminalOptions::new(
        "/nonexistent-selected-helper".into(),
        None,
        entries.clone(),
    )
    .unwrap();
    let contexts = Arc::new(NativeMcpContexts::new());
    let options = NativeReferenceHostMcpOptions::from_captured_startup(
        &roots,
        &terminal,
        contexts.clone(),
        Some((McpResolverConfig::literal_only(), [7; 32])),
    )
    .unwrap();
    let startup = options.startup.as_ref().unwrap();
    assert!(Arc::ptr_eq(&options.contexts, &contexts));
    assert!(Arc::ptr_eq(&options.clock, &startup.clock));
    assert_eq!(startup.peer_lifetime, McpPeerLifetime::OwnerControlled);
    assert_eq!(startup.captured_environment, entries);
    assert!(!startup.owner_cancellation.is_cancelled());
    let config = crate::mcp::config::McpConfig::decode(
        br#"{"mcp":{"selected":{"command":"nonexistent-selected-server"}}}"#,
    )
    .unwrap();
    let mut factory = startup
        .stdio
        .as_ref()
        .unwrap()
        .factory(Arc::new(config.servers()[0].clone()))
        .unwrap();
    let launch = factory.launch().unwrap();
    let retained = launch.cwd_for_test().metadata().unwrap();
    assert_eq!(
        (retained.dev(), retained.ino()),
        (original.dev(), original.ino())
    );
    assert_eq!(launch.environment_for_test().entries(), entries);
    let workers = crate::NativeOwnedWorkerScope::new();
    drop(launch.connect(
        workers.clone(),
        startup.catalog_epoch + std::time::Duration::from_secs(1),
        CancellationToken::new(),
        Box::new(()),
    ));
    workers.close();
    assert!(workers.completion().is_complete());
    startup.owner_cancellation.cancel();
    assert!(
        startup
            .network
            .as_ref()
            .unwrap()
            .owner_cancellation()
            .is_cancelled()
    );
    let offline =
        NativeReferenceHostMcpOptions::from_captured_startup(&roots, &terminal, contexts, None)
            .unwrap();
    let offline = offline.startup.unwrap();
    assert!(offline.network.is_none());
    assert!(offline.stdio.is_some());
}

#[test]
fn production_trust_is_bundled_and_debug_is_redacted() {
    assert_eq!(
        format!("{:?}", super::super::startup::bundled_trust().unwrap()),
        "McpHttpTrust { <redacted> }"
    );
}
