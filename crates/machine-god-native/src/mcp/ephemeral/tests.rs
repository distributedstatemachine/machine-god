use super::*;
use crate::mcp::{
    context::NativeMcpContexts,
    runtime::{
        NativeMcpRuntimeLimits, NativeMcpRuntimeToolCall, NativeMcpToolExecutionPolicy,
        NativeMcpToolExecutor,
    },
};
use futures_executor::block_on;
use machine_god_core::{BoxFuture, ToolError, ToolExecution};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

const STDIO: &[u8] = br#"[{"name":"stdio","command":"/missing/server","args":[],"env":[]}]"#;

fn http_selection(urls: &[&str], include_stdio: bool) -> Vec<u8> {
    let mut servers = urls
        .iter()
        .enumerate()
        .map(|(index, url)| {
            serde_json::json!({"name":format!("h{index}"), "type":"http", "url":url, "headers":[]})
        })
        .collect::<Vec<_>>();
    if include_stdio {
        servers
            .push(serde_json::json!({"name":"stdio", "command":"/bin/echo", "args":[], "env":[]}));
    }
    serde_json::to_vec(&servers).unwrap()
}

#[test]
fn network_requirement_is_pure_and_covers_all_selected_transports() {
    use NativeMcpNetworkRequirement::{LiteralOnly, None, SystemDns};
    assert_eq!(empty().network_requirement(), None);
    assert_eq!(
        NativeMcpEphemeralConfiguration::decode(Some(b"[]"))
            .unwrap()
            .network_requirement(),
        None
    );
    for (urls, expected) in [
        (vec![], None),
        (vec!["https://127.0.0.1/mcp"], LiteralOnly),
        (vec!["https://[::1]/mcp"], LiteralOnly),
        (vec!["https://LOCALHOST/mcp"], LiteralOnly),
        (vec!["HTTP://LoCaLhOsT:8123/mcp"], LiteralOnly),
        (vec!["https://%6cocalhost/mcp"], LiteralOnly),
        (vec!["https://localhost./mcp"], SystemDns),
        (vec!["https://localhost.example/mcp"], SystemDns),
        (vec!["https://EXAMPLE.test/mcp"], SystemDns),
        (
            vec!["https://[2001:db8::1]/mcp", "https://192.0.2.1/mcp"],
            LiteralOnly,
        ),
        (
            vec!["https://localhost/mcp", "https://example.test/mcp"],
            SystemDns,
        ),
        (
            vec!["https://example.test/mcp", "https://localhost/mcp"],
            SystemDns,
        ),
    ] {
        for include_stdio in [false, true] {
            let bytes = http_selection(&urls, include_stdio);
            let selected = NativeMcpEphemeralConfiguration::decode(Some(&bytes)).unwrap();
            assert_eq!(selected.network_requirement(), expected);
            assert_eq!(selected.clone().network_requirement(), expected);
        }
    }
}

#[test]
fn invalid_http_endpoint_is_rejected_even_after_system_dns_is_required() {
    // These satisfy the initial structural HTTPS shape but must not survive
    // the established endpoint parser used to classify captured authority.
    for invalid in [
        "https://[not-ip]/mcp",
        "https://example.test:invalid/mcp",
        "https://example.test:65536/mcp",
        "https://example.test/%zz",
        "https://%ff/mcp",
    ] {
        for urls in [vec![invalid], vec!["https://example.test/mcp", invalid]] {
            let bytes = http_selection(&urls, true);
            assert_eq!(
                NativeMcpEphemeralConfiguration::decode(Some(&bytes)).unwrap_err(),
                McpConfigError::Invalid
            );
        }
    }
}

fn empty() -> NativeMcpEphemeralConfiguration {
    NativeMcpEphemeralConfiguration::decode(None).unwrap()
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

#[test]
fn absent_and_empty_are_authoritative_but_null_and_profile_grammar_are_invalid() {
    assert!(empty().is_empty());
    assert!(
        NativeMcpEphemeralConfiguration::decode(Some(b"[]"))
            .unwrap()
            .is_empty()
    );
    for bytes in [
        b"null".as_slice(),
        b"{}",
        b"{\"mcp\":{}}",
        b"[null]",
        b"true",
    ] {
        assert!(NativeMcpEphemeralConfiguration::decode(Some(bytes)).is_err());
    }
}

#[test]
fn stdio_admission_is_required_absolute_literal_and_redacted() {
    let selected = NativeMcpEphemeralConfiguration::decode(Some(br#"[{"name":"stdio","command":"/bin/echo","args":["","$TOKEN"],"env":[{"name":"TOKEN","value":"$LITERAL_SECRET"}]}]"#)).unwrap();
    assert_eq!(selected.server_count(), 1);
    let server = selected.configuration.server("stdio").unwrap();
    assert!(server.required() && server.enabled());
    assert_eq!(
        server.environment().get("TOKEN").unwrap().as_ref(),
        "$LITERAL_SECRET"
    );
    let super::super::config::McpTransportConfig::Stdio(stdio) = server.transport() else {
        panic!("stdio")
    };
    assert_eq!(
        stdio
            .args()
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<&str>>(),
        ["", "$TOKEN"]
    );
    assert!(!format!("{selected:?}").contains("SECRET"));
    assert!(selected.identities[0].starts_with(b"MG-ACP-MCP-1\0"));
    for bytes in [
        br#"[{"name":"s","command":"relative","args":[],"env":[]}]"#.as_slice(),
        br#"[{"name":"s","command":"/bin/s","env":[]}]"#,
        br#"[{"name":"s","command":"/bin/s","args":[]}]"#,
        br#"[{"name":"s","command":"/bin/s","args":[],"env":[{"name":"A","value":"1"},{"name":"A","value":"2"}]}]"#,
        br#"[{"name":"s","command":"/bin/s","args":[],"env":[{"name":"1A","value":"1"}]}]"#,
        br#"[{"name":"s","command":"/bin/s","args":[],"env":[],"required":false}]"#,
        br#"[{"name":"s","name":"other","command":"/bin/s","args":[],"env":[]}]"#,
    ] { assert!(NativeMcpEphemeralConfiguration::decode(Some(bytes)).is_err(), "{bytes:?}"); }
}

#[test]
fn http_resolved_authorization_is_not_a_profile_header_or_credential_selection() {
    let selected = NativeMcpEphemeralConfiguration::decode(Some(br#"[{"name":"h","type":"http","url":"https://example.test/mcp","headers":[{"name":"Authorization","value":"Bearer SECRET"},{"name":"X-Tab","value":"\t"}]}]"#)).unwrap();
    let super::super::config::McpTransportConfig::Http(remote) =
        selected.configuration.server("h").unwrap().transport()
    else {
        panic!("HTTP")
    };
    assert!(remote.headers().is_empty());
    assert!(remote.bearer_token_env().is_none());
    assert!(remote.oauth().is_none());
    #[cfg(feature = "mcp-http")]
    assert_eq!(
        selected.headers[0].1.iter().collect::<Vec<_>>(),
        [
            ("Authorization", b"Bearer SECRET".as_slice()),
            ("X-Tab", b"\t")
        ]
    );
    for bytes in [
        br#"[{"name":"h","type":"sse","url":"https://example.test/mcp","headers":[]}]"#.as_slice(),
        br#"[{"name":"h","type":"http","url":"http://example.test:80/mcp","headers":[]}]"#,
        br#"[{"name":"h","type":"http","url":"http://localhost/mcp","headers":[]}]"#,
        br#"[{"name":"h","type":"http","url":"https://example.test/mcp"}]"#,
        br#"[{"name":"h","type":"http","url":"https://example.test/mcp","headers":[{"name":"Authorization","value":"1"},{"name":"authorization","value":"2"}]}]"#,
        br#"[{"name":"h","type":"http","url":"https://example.test/mcp","headers":[{"name":"MCP-Param-X","value":"1"}]}]"#,
        br#"[{"name":"h","type":"http","url":"https://example.test/mcp","headers":[{"name":"Host","value":"1"}]}]"#,
        br#"[{"name":"h","type":"http","url":"https://example.test/mcp","headers":[],"oauth":{}}]"#,
    ] { assert!(NativeMcpEphemeralConfiguration::decode(Some(bytes)).is_err(), "{bytes:?}"); }
}

#[test]
fn bounded_input_rejects_oversize_duplicates_and_deep_trees() {
    let many = serde_json::Value::Array((0..65).map(|i| serde_json::json!({"name":format!("s{i}"), "command":"/bin/s", "args":[], "env":[]})).collect());
    assert_eq!(
        NativeMcpEphemeralConfiguration::decode(Some(&serde_json::to_vec(&many).unwrap()))
            .unwrap_err(),
        McpConfigError::Limit
    );
    assert!(
        NativeMcpEphemeralConfiguration::decode(Some(&vec![
            b' ';
            super::super::config::MAX_CONFIG_BYTES
                + 1
        ]))
        .is_err()
    );
    assert!(NativeMcpEphemeralConfiguration::decode(Some(b"[[[[[[[[[[[]]]]]]]]]]]")).is_err());
    let duplicate = [
        STDIO[..STDIO.len() - 1].to_vec(),
        b",".to_vec(),
        STDIO[1..].to_vec(),
    ]
    .concat();
    assert_eq!(
        NativeMcpEphemeralConfiguration::decode(Some(&duplicate)).unwrap_err(),
        McpConfigError::AlreadyExists
    );
}

#[test]
fn literal_environment_and_arguments_preserve_newlines_but_reject_nul() {
    let selected = NativeMcpEphemeralConfiguration::decode(Some(br#"[{"name":"s","command":"/bin/sh","args":["-c","echo a\necho b"],"env":[{"name":"LITERAL","value":"line1\nline2\t$TOKEN"}]}]"#)).unwrap();
    let server = selected.configuration.server("s").unwrap();
    assert_eq!(
        server.environment().get("LITERAL").unwrap().as_ref(),
        "line1\nline2\t$TOKEN"
    );
    let super::super::config::McpTransportConfig::Stdio(stdio) = server.transport() else {
        panic!("stdio")
    };
    assert_eq!(stdio.args()[1].as_ref(), "echo a\necho b");
    for bytes in [
        br#"[{"name":"s","command":"/bin/sh","args":["\u0000"],"env":[]}]"#.as_slice(),
        br#"[{"name":"s","command":"/bin/sh","args":[],"env":[{"name":"A","value":"\u0000"}]}]"#,
    ] {
        assert!(NativeMcpEphemeralConfiguration::decode(Some(bytes)).is_err());
    }
}

#[derive(Default)]
pub(super) struct Clock {
    calls: AtomicUsize,
    hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let hook = self.hook.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
        Instant::now()
    }
    fn sleep_until(&self, _: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}
struct NeverExecute;
impl NativeMcpToolExecutor for NeverExecute {
    fn execute(
        &self,
        _: NativeMcpRuntimeToolCall,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        panic!("admission cannot execute tools")
    }
}
pub(super) fn options() -> (NativeMcpEphemeralOptions, Arc<Clock>) {
    let clock = Arc::new(Clock::default());
    let runtime = Arc::new(
        NativeMcpRuntime::new(
            Arc::new(NativeMcpContexts::new()),
            clock.clone(),
            Arc::new(NeverExecute),
            NativeMcpToolExecutionPolicy::default(),
            NativeMcpRuntimeLimits::default(),
        )
        .unwrap(),
    );
    (
        NativeMcpEphemeralOptions {
            runtime,
            workers: NativeOwnedWorkerScope::new(),
            reserved_tool_names: Box::new([]),
            captured_environment: Vec::new(),
            stdio: None,
            clock: clock.clone(),
            catalog_epoch: Instant::now(),
            owner_cancellation: CancellationToken::new(),
            #[cfg(feature = "mcp-http")]
            network: None,
            peer_lifetime: McpPeerLifetime::OwnerControlled,
            max_retained_bytes: 16 * 1024 * 1024,
            max_retained_generations: 4,
        },
        clock,
    )
}

#[test]
fn construction_and_unpolled_operations_are_inert_and_empty_selection_becomes_ready() {
    let (options, clock) = options();
    let runtime = options.runtime.clone();
    let workers = options.workers.clone();
    let owner = NativeMcpEphemeralOwner::new(options).unwrap();
    drop(owner.replace(empty(), CancellationToken::new(), deadline()));
    drop(owner.settle(CancellationToken::new(), deadline()));
    assert_eq!(clock.calls.load(Ordering::Relaxed), 0);
    assert!(owner.ready().is_err());
    assert!(runtime.publication_checkpoint().unwrap().is_unpublished());
    let receipt = block_on(owner.replace(empty(), CancellationToken::new(), deadline())).unwrap();
    assert!(receipt.startup().required_ready());
    assert!(!receipt.closed_after_publication());
    owner.ready().unwrap();
    block_on(owner.settle(CancellationToken::new(), deadline())).unwrap();
    assert!(owner.ready().is_err());
    assert!(receipt.cleanup_complete());
    workers.close();
    assert!(workers.completion().is_complete());
}

#[test]
fn required_failure_and_cancellation_preserve_exact_old_selection() {
    let (options, _) = options();
    let runtime = options.runtime.clone();
    let owner = NativeMcpEphemeralOwner::new(options).unwrap();
    block_on(owner.replace(empty(), CancellationToken::new(), deadline())).unwrap();
    let before = runtime.publication_checkpoint().unwrap();
    let failed = block_on(owner.replace(
        NativeMcpEphemeralConfiguration::decode(Some(STDIO)).unwrap(),
        CancellationToken::new(),
        deadline(),
    ));
    assert!(matches!(failed, Err(NativeMcpEphemeralError::Startup(_))));
    assert!(
        runtime
            .publication_checkpoint()
            .unwrap()
            .same_selection(&before)
    );
    owner.ready().unwrap();
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert_eq!(
        block_on(owner.replace(empty(), cancelled, deadline())).unwrap_err(),
        NativeMcpEphemeralError::Cancelled
    );
    assert!(
        runtime
            .publication_checkpoint()
            .unwrap()
            .same_selection(&before)
    );
    owner.ready().unwrap();
    block_on(owner.settle(CancellationToken::new(), deadline())).unwrap();
}

#[test]
fn stale_replacement_does_not_overwrite_external_publication() {
    let (options, _) = options();
    let runtime = options.runtime.clone();
    let owner = NativeMcpEphemeralOwner::new(options).unwrap();
    block_on(owner.replace(empty(), CancellationToken::new(), deadline())).unwrap();
    runtime
        .publish(runtime.prepare_candidate(vec![], &[]).unwrap())
        .unwrap();
    let external = runtime.publication_checkpoint().unwrap();
    assert!(block_on(owner.replace(empty(), CancellationToken::new(), deadline())).is_err());
    assert!(
        runtime
            .publication_checkpoint()
            .unwrap()
            .same_selection(&external)
    );
    assert!(owner.ready().is_err());
}

#[test]
fn receipt_retention_is_bounded_and_released_generations_can_be_reused() {
    let (mut options, _) = options();
    options.max_retained_generations = 2;
    let owner = NativeMcpEphemeralOwner::new(options).unwrap();
    let first = block_on(owner.replace(empty(), CancellationToken::new(), deadline())).unwrap();
    let second = block_on(owner.replace(empty(), CancellationToken::new(), deadline())).unwrap();
    assert_eq!(
        block_on(owner.replace(empty(), CancellationToken::new(), deadline())).unwrap_err(),
        NativeMcpEphemeralError::Limit
    );
    owner.ready().unwrap();
    drop(first);
    block_on(owner.replace(empty(), CancellationToken::new(), deadline())).unwrap();
    drop(second);
    block_on(owner.settle(CancellationToken::new(), deadline())).unwrap();
}

#[test]
fn independent_sessions_do_not_share_empty_publication_or_close_authority() {
    let first = NativeMcpEphemeralOwner::new(options().0).unwrap();
    let second = NativeMcpEphemeralOwner::new(options().0).unwrap();
    block_on(first.replace(empty(), CancellationToken::new(), deadline())).unwrap();
    block_on(second.replace(empty(), CancellationToken::new(), deadline())).unwrap();
    first.close();
    assert!(first.ready().is_err());
    second.ready().unwrap();
    block_on(second.replace(empty(), CancellationToken::new(), deadline())).unwrap();
}

#[test]
fn unpublished_runtime_cannot_be_claimed_by_two_ephemeral_owners() {
    let (selected, _) = options();
    let runtime = selected.runtime.clone();
    let first = NativeMcpEphemeralOwner::new(selected).unwrap();
    let (mut second, _) = options();
    second.runtime = runtime;
    assert!(NativeMcpEphemeralOwner::new(second).is_err());
    block_on(first.replace(empty(), CancellationToken::new(), deadline())).unwrap();
    first.ready().unwrap();
}

#[test]
fn profile_and_ephemeral_runtime_bindings_are_mutually_exclusive() {
    for profile_first in [true, false] {
        let (selected, _) = options();
        let runtime = selected.runtime.clone();
        let controller = profile_controller(runtime.clone());
        if profile_first {
            runtime.bind_controller(&controller).unwrap();
            assert!(NativeMcpEphemeralOwner::new(selected).is_err());
        } else {
            let owner = NativeMcpEphemeralOwner::new(selected).unwrap();
            assert!(runtime.bind_controller(&controller).is_err());
            block_on(owner.replace(empty(), CancellationToken::new(), deadline())).unwrap();
            owner.ready().unwrap();
        }
    }
}

fn profile_controller(
    runtime: Arc<NativeMcpRuntime>,
) -> Arc<super::super::controller::NativeMcpController> {
    use super::super::{
        controller::{
            NativeMcpController, NativeMcpControllerOptions, NativeMcpControllerStartupOptions,
        },
        management::NativeMcpManagementService,
        store::NativeMcpConfigStore,
    };
    let (selected, _) = options();
    Arc::new(
        NativeMcpController::new(NativeMcpControllerOptions {
            runtime,
            management: Arc::new(NativeMcpManagementService::new(Arc::new(
                NativeMcpConfigStore::new("/acp-sentinel-never-loaded".into()).unwrap(),
            ))),
            workers: selected.workers,
            reserved_tool_names: Box::new([]),
            startup: NativeMcpControllerStartupOptions {
                captured_environment: vec![],
                stdio: None,
                clock: selected.clock,
                catalog_epoch: selected.catalog_epoch,
                owner_cancellation: selected.owner_cancellation,
                #[cfg(feature = "mcp-http")]
                network: None,
                #[cfg(feature = "mcp-http")]
                authentication: vec![],
                peer_lifetime: McpPeerLifetime::OwnerControlled,
                max_retained_bytes: 16 * 1024 * 1024,
            },
            #[cfg(feature = "mcp-http")]
            stored_authentication: None,
            max_retained_generations: 4,
        })
        .unwrap(),
    )
}

#[test]
fn preexisting_publication_and_clock_reentrant_cancellation_are_rejected() {
    let (options, _) = options();
    options
        .runtime
        .publish(options.runtime.prepare_candidate(vec![], &[]).unwrap())
        .unwrap();
    assert_eq!(
        NativeMcpEphemeralOwner::new(options).unwrap_err(),
        NativeMcpEphemeralError::Invalid
    );
    let (options, clock) = self::options();
    let runtime = options.runtime.clone();
    let cancellation = CancellationToken::new();
    let cancelled = cancellation.clone();
    *clock.hook.lock().unwrap() = Some(Box::new(move || {
        cancelled.cancel();
    }));
    let owner = NativeMcpEphemeralOwner::new(options).unwrap();
    assert_eq!(
        block_on(owner.replace(empty(), cancellation, deadline())).unwrap_err(),
        NativeMcpEphemeralError::Cancelled
    );
    assert!(runtime.publication_checkpoint().unwrap().is_unpublished());
}
