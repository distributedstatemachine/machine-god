use super::*;
use crate::NativeOwnedWorkerScope;
use crate::mcp::config::McpConfig;
use machine_god_core::CancellationToken;
use std::time::{Duration, Instant};

fn cwd() -> Arc<File> {
    // Deliberately not a directory: construction must not inspect its descriptor.
    Arc::new(File::open("/dev/null").unwrap())
}
fn startup(environment: Vec<(OsString, OsString)>) -> NativeMcpStdioStartup {
    let startup = NativeMcpStdioStartup::new(
        "/nonexistent/mcp-helper".into(),
        vec!["--selected-helper".into()],
        environment,
        cwd(),
        WireLimits::default(),
    )
    .unwrap();
    #[cfg(target_os = "macos")]
    let startup = startup
        .with_process_inventory_service(
            "/nonexistent/inventory-helper".into(),
            vec!["--selected-inventory".into()],
        )
        .unwrap();
    startup
}
fn server(json: &str) -> Arc<McpServerConfig> {
    let config = McpConfig::decode(json.as_bytes()).unwrap();
    Arc::new(config.servers()[0].clone())
}
fn basic() -> Arc<McpServerConfig> {
    server(
        r#"{"mcp":{"fixture":{"command":["nonexistent-target","", "$TOKEN", "$(literal)","*.txt"]}}}"#,
    )
}

#[test]
fn constructors_factories_and_unpolled_connection_are_inert() {
    let startup = startup(vec![]);
    let mut factory = startup.factory(basic()).unwrap();
    let launch = factory.launch().unwrap();
    let host = NativeOwnedWorkerScope::new();
    let completion = host.completion();
    drop(launch.connect(
        host.clone(),
        Instant::now() + Duration::from_secs(1),
        CancellationToken::new(),
        Box::new(()),
    ));
    host.close();
    assert!(completion.is_complete());
    #[cfg(target_os = "macos")]
    assert_eq!(
        startup
            .helper
            .inventory_helper()
            .unwrap()
            .service_spawn_count_for_test(),
        0
    );
}

#[test]
fn exact_snapshot_and_argv_remain_shared_across_factory_launches() {
    let startup = startup(vec![("TOKEN".into(), "captured-secret".into())]);
    let snapshot = basic();
    let mut factory = startup.factory(snapshot.clone()).unwrap();
    assert!(Arc::ptr_eq(factory.configuration(), &snapshot));
    let first = factory.launch().unwrap();
    let second = factory.launch().unwrap();
    let third = factory.clone().launch().unwrap();
    assert!(first.shares_storage_with(&second));
    assert!(first.shares_storage_with(&third));
    assert!(Arc::ptr_eq(first.cwd_for_test(), &startup.cwd));
    assert_eq!(
        first.argv_for_test(),
        (
            "nonexistent-target",
            &[
                String::new(),
                "$TOKEN".into(),
                "$(literal)".into(),
                "*.txt".into()
            ][..]
        )
    );
    assert!(
        first
            .environment_for_test()
            .shares_storage_with(&startup.environment)
    );
    drop(startup);
    assert!(first.shares_storage_with(&factory.launch().unwrap()));
    for debug in [format!("{factory:?}"), format!("{first:?}")] {
        assert!(!debug.contains("captured-secret"));
        assert!(!debug.contains("nonexistent-target"));
    }
}

#[test]
fn restart_cloning_and_invalid_raw_command_do_not_allocate() {
    let authority = startup(vec![("TOKEN".into(), "secret".into())]);
    let mut factory = authority.factory(basic()).unwrap();
    allocation_counter::measure(|| {});
    let cloned = allocation_counter::measure(|| {
        drop(factory.launch().unwrap());
    });
    assert_eq!(cloned.count_total, 0);

    let helper = PathBuf::from("/missing/helper");
    let command = "x".repeat(4097);
    let directory = cwd();
    let environment = vec![(OsString::from("TOKEN"), OsString::from("selected"))];
    let rejected = allocation_counter::measure(|| {
        assert!(
            McpStdioLaunch::new(
                helper,
                vec![],
                command,
                vec![],
                environment,
                None,
                directory,
                WireLimits::default()
            )
            .is_err()
        );
    });
    assert_eq!(rejected.count_total, 0);
}

#[test]
fn nonempty_configuration_replaces_inherited_environment_without_expansion() {
    let startup = startup(vec![
        ("TOKEN".into(), "parent-value".into()),
        ("HOME".into(), "/parent/home".into()),
        ("PATH".into(), "/parent/bin:relative/bin".into()),
    ]);
    let configuration = server(
        r#"{"mcp":{"fixture":{"command":"target","environment":{"TOKEN":"$HOME/${TOKEN}","PATH":"/child/bin","EMPTY":""},"env":{"IGNORED":"alias"}}}}"#,
    );
    let factory = startup.factory(configuration).unwrap();
    let entries = factory.template.environment_for_test().entries();
    assert_eq!(entries.len(), 3);
    assert!(entries.contains(&("TOKEN".into(), "$HOME/${TOKEN}".into())));
    assert!(entries.contains(&("PATH".into(), "/child/bin".into())));
    assert!(entries.contains(&("EMPTY".into(), "".into())));
    assert!(
        !entries
            .iter()
            .any(|(key, _)| key == "HOME" || key == "IGNORED")
    );
    assert_eq!(
        startup.search_path.as_deref(),
        Some(OsStr::new("/parent/bin:relative/bin"))
    );
}

#[test]
fn empty_explicit_environment_inherits_only_selected_snapshot() {
    let startup = startup(vec![(
        "SNAPSHOT".into(),
        OsString::from_vec(vec![0xff, b'x']),
    )]);
    let configuration = server(
        r#"{"mcp":{"fixture":{"command":"target","environment":{},"env":{"IGNORED":"alias"}}}}"#,
    );
    let factory = startup.factory(configuration).unwrap();
    assert!(
        factory
            .template
            .environment_for_test()
            .shares_storage_with(&startup.environment)
    );
    assert_eq!(
        factory.template.environment_for_test().entries()[0]
            .1
            .as_bytes(),
        &[0xff, b'x']
    );
}

#[test]
fn replacement_environment_is_bounded_independently_not_merged() {
    let authority = startup(
        (0..15)
            .map(|i| (format!("PARENT_{i}").into(), "x".repeat(16 * 1024).into()))
            .collect(),
    );
    let small =
        server(r#"{"mcp":{"fixture":{"command":"target","environment":{"ONLY":"selected"}}}}"#);
    assert_eq!(
        authority
            .factory(small)
            .unwrap()
            .template
            .environment_for_test()
            .entries()
            .len(),
        1
    );
    let entries = (0..17)
        .map(|i| format!("\"K{i}\":\"{}\"", "x".repeat(16 * 1024)))
        .collect::<Vec<_>>()
        .join(",");
    let large = server(&format!(
        "{{\"mcp\":{{\"fixture\":{{\"command\":\"target\",\"env\":{{{entries}}}}}}}}}"
    ));
    assert!(authority.factory(large).is_err());
}

#[test]
fn lookup_path_preserves_unix_bytes_and_pinned_empty_segment_policy() {
    let missing = startup(vec![]);
    assert_eq!(
        missing.search_path.as_deref(),
        Some(OsStr::new(PINNED_DEFAULT_PATH))
    );
    let empty = startup(vec![("PATH".into(), ":::".into())]);
    assert!(empty.search_path.is_none());
    let selected = startup(vec![(
        "PATH".into(),
        OsString::from_vec(b":/bin::relative/\xff:".to_vec()),
    )]);
    assert_eq!(
        selected.search_path.as_deref().unwrap().as_bytes(),
        b"/bin:relative/\xff"
    );
    assert_eq!(
        selected.environment.entries()[0].1.as_bytes(),
        b":/bin::relative/\xff:"
    );
    assert!(selected.factory(basic()).is_ok());
}

#[test]
fn captured_environment_and_path_bounds_fail_without_effects() {
    for environment in [
        vec![("DUP".into(), "a".into()), ("DUP".into(), "b".into())],
        vec![("BAD=KEY".into(), "value".into())],
        vec![("PATH".into(), "x".repeat(8193).into())],
        vec![("PATH".into(), ":".repeat(256).into())],
        (0..513)
            .map(|i| (format!("K{i}").into(), "v".into()))
            .collect(),
        (0..17)
            .map(|i| (format!("K{i}").into(), "x".repeat(16 * 1024).into()))
            .collect(),
    ] {
        assert!(
            NativeMcpStdioStartup::new(
                "/missing/helper".into(),
                vec![],
                environment,
                cwd(),
                WireLimits::default()
            )
            .is_err()
        );
    }
    assert!(
        NativeMcpStdioStartup::new(
            "relative-helper".into(),
            vec![],
            vec![],
            cwd(),
            WireLimits::default()
        )
        .is_err()
    );
    assert!(
        NativeMcpStdioStartup::new(
            "/missing/helper".into(),
            vec![],
            vec![],
            cwd(),
            WireLimits {
                max_depth: 0,
                ..WireLimits::default()
            }
        )
        .is_err()
    );
}

#[test]
fn disabled_and_remote_configurations_cannot_mint_stdio_factories() {
    let startup = startup(vec![]);
    for json in [
        r#"{"mcp":{"fixture":{"command":"target","enabled":false}}}"#,
        r#"{"mcp":{"fixture":{"type":"http","url":"https://example.com/mcp"}}}"#,
    ] {
        assert!(startup.factory(server(json)).is_err());
    }
    // Deprecated HTTP+SSE cannot reach factory selection at all.
    assert!(
        McpConfig::decode(br#"{"mcp":{"fixture":{"type":"sse","url":"https://example.com/mcp"}}}"#)
            .is_err()
    );
}

#[test]
fn all_server_factories_share_the_same_selected_helper_and_cwd() {
    let startup = startup(vec![]);
    let mut first = startup.factory(basic()).unwrap();
    let mut second = startup
        .factory(server(
            r#"{"mcp":{"other":{"command":"other-target","env":{"TOKEN":"isolated"}}}}"#,
        ))
        .unwrap();
    let left = first.launch().unwrap();
    let right = second.launch().unwrap();
    assert!(Arc::ptr_eq(
        left.selected_helper_for_test(),
        right.selected_helper_for_test()
    ));
    assert!(Arc::ptr_eq(left.cwd_for_test(), right.cwd_for_test()));
    assert!(
        !left
            .environment_for_test()
            .shares_storage_with(right.environment_for_test())
    );
    #[cfg(target_os = "macos")]
    assert_eq!(
        left.selected_helper_for_test()
            .inventory_helper()
            .unwrap()
            .service_spawn_count_for_test(),
        0
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_requires_one_inert_service_selection_and_rejects_rebinding() {
    let authority = NativeMcpStdioStartup::new(
        "/missing/helper".into(),
        vec![],
        vec![],
        cwd(),
        WireLimits::default(),
    )
    .unwrap();
    assert!(authority.factory(basic()).is_err());
    let selected = authority
        .with_process_inventory_service("/missing/inventory".into(), vec![])
        .unwrap();
    assert!(selected.factory(basic()).is_ok());
    assert!(
        selected
            .with_process_inventory_service("/another/inventory".into(), vec![])
            .is_err()
    );
}
