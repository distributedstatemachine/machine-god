use super::*;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    base: PathBuf,
    store: Arc<NativeMcpConfigStore>,
    service: NativeMcpManagementService,
}

impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "mg-mcp-management-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&base).unwrap();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        let base = fs::canonicalize(base).unwrap();
        let store = Arc::new(NativeMcpConfigStore::new(base.join("profile")).unwrap());
        let service = NativeMcpManagementService::new(Arc::clone(&store));
        Self {
            base,
            store,
            service,
        }
    }

    fn execute(&self, command: McpCommand) -> Result<NativeMcpManagementReceipt> {
        self.service.execute(command, &CancellationToken::new())
    }

    fn seed(&self, bytes: &[u8]) {
        fs::create_dir(self.base.join("profile")).unwrap();
        fs::set_permissions(self.base.join("profile"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(self.store.path(), bytes).unwrap();
        fs::set_permissions(self.store.path(), fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn no_profile(&self) {
        assert!(!self.base.join("profile").exists());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.base).unwrap();
    }
}

fn add(server: &str, command: &str, arguments: &[&str]) -> McpCommand {
    McpCommand::Add {
        server: server.into(),
        command: command.into(),
        arguments: arguments.iter().map(|value| (*value).into()).collect(),
    }
}

fn saved(receipt: NativeMcpManagementReceipt) -> NativeMcpConfigCommit {
    assert!(!receipt.failed());
    let NativeMcpManagementReceipt::Saved { commit, activation } = receipt else {
        panic!("expected save receipt");
    };
    assert_eq!(activation, McpManagementActivation::NotAttempted);
    assert_eq!(commit.durability(), McpConfigCommitDurability::Confirmed);
    commit
}

#[test]
fn construction_path_and_missing_listing_create_nothing() {
    let fixture = Fixture::new();
    fixture.no_profile();
    let path = fixture.execute(McpCommand::Path).unwrap();
    assert!(!path.failed());
    assert!(matches!(path, NativeMcpManagementReceipt::Path(path) if path == fixture.store.path()));
    for command in [McpCommand::Summary, McpCommand::List] {
        let receipt = fixture.execute(command).unwrap();
        assert!(!receipt.failed());
        assert!(
            matches!(receipt, NativeMcpManagementReceipt::Configured(servers) if servers.is_empty())
        );
    }
    fixture.no_profile();
}

#[test]
fn path_does_not_load_invalid_configuration() {
    let fixture = Fixture::new();
    fixture.seed(b"not json");
    assert!(matches!(
        fixture.execute(McpCommand::Path),
        Ok(NativeMcpManagementReceipt::Path(_))
    ));
    assert!(matches!(
        fixture.execute(McpCommand::Summary),
        Err(NativeMcpManagementError::Store(_))
    ));
    assert_eq!(fs::read(fixture.store.path()).unwrap(), b"not json");
}

#[test]
fn configured_projection_preserves_order_and_only_metadata() {
    let fixture = Fixture::new();
    fixture.seed(br#"{"mcp":{"local":{"command":"secret-command","args":["secret-argument"],"env":{"TOKEN":"secret-value"}},"remote":{"type":"http","url":"https://example.com/secret-path","enabled":false,"required":true},"events":{"type":"sse","url":"https://example.com/events"}}}"#);
    let receipt = fixture.execute(McpCommand::List).unwrap();
    assert_eq!(
        format!("{receipt:?}"),
        "NativeMcpManagementReceipt { <redacted> }"
    );
    let NativeMcpManagementReceipt::Configured(servers) = receipt else {
        panic!("expected metadata")
    };
    assert_eq!(servers.len(), 3);
    assert_eq!(&*servers[0].name, "local");
    assert_eq!(servers[0].transport, McpConfiguredTransport::Stdio);
    assert!(servers[0].enabled);
    assert!(!servers[0].required);
    assert_eq!(servers[1].transport, McpConfiguredTransport::Http);
    assert!(!servers[1].enabled);
    assert!(servers[1].required);
    assert_eq!(servers[2].transport, McpConfiguredTransport::Sse);
    assert_eq!(
        format!("{:?}", servers[0]),
        "McpConfiguredServer { <redacted> }"
    );
    assert!(!fixture.base.join("profile/.mcp.lock").exists());
}

#[test]
fn add_replaces_deliberately_preserving_position_and_literal_arguments() {
    let fixture = Fixture::new();
    saved(fixture.execute(add("first", "old", &[])).unwrap());
    saved(fixture.execute(add("second", "other", &[])).unwrap());
    let args = ["$(secret)", "`literal`", "a;b", "'quote'", "$TOKEN", "*.rs"];
    let receipt = saved(fixture.execute(add("first", "new", &args)).unwrap());
    assert!(receipt.changed());
    assert_eq!(
        receipt
            .intended()
            .servers()
            .iter()
            .map(McpServerConfig::name)
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    let config = receipt.intended().server("first").unwrap();
    assert!(config.enabled());
    assert!(!config.required());
    assert_eq!(config.startup_timeout_ms(), 10_000);
    assert_eq!(config.operation_timeout_ms(), 60_000);
    assert!(config.environment().is_empty());
    let McpTransportConfig::Stdio(stdio) = config.transport() else {
        panic!("expected stdio")
    };
    assert_eq!(stdio.command(), "new");
    assert_eq!(
        stdio
            .args()
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<&str>>(),
        args
    );
    assert_eq!(stdio.restart_limit(), 1);
    assert_eq!(fixture.store.load().unwrap().config(), receipt.intended());
}

#[test]
fn replacement_restores_stdio_defaults_instead_of_merging_remote_config() {
    let fixture = Fixture::new();
    fixture.seed(br#"{"mcp":{"server":{"type":"http","url":"https://example.com","enabled":false,"required":true,"env":{"TOKEN":"secret"}}}}"#);
    let receipt = saved(
        fixture
            .execute(add("server", "literal-command", &[]))
            .unwrap(),
    );
    let config = receipt.intended().server("server").unwrap();
    assert!(config.enabled());
    assert!(!config.required());
    assert!(config.environment().is_empty());
    assert!(matches!(config.transport(), McpTransportConfig::Stdio(_)));
}

#[test]
fn remove_is_exact_and_missing_noop_creates_nothing() {
    let fixture = Fixture::new();
    let remove = |name: &str| McpCommand::Remove {
        server: name.into(),
    };
    assert!(!saved(fixture.execute(remove("missing")).unwrap()).changed());
    fixture.no_profile();
    saved(fixture.execute(add("Name", "node", &[])).unwrap());
    assert!(!saved(fixture.execute(remove("name")).unwrap()).changed());
    assert!(saved(fixture.execute(remove("Name")).unwrap()).changed());
    assert!(fixture.store.load().unwrap().config().servers().is_empty());
}

#[test]
fn identical_replace_preserves_existing_bytes_and_creates_no_lock() {
    let fixture = Fixture::new();
    let bytes = br#"{ "mcp": { "server": {"command":"node"} } }"#;
    fixture.seed(bytes);
    assert!(!saved(fixture.execute(add("server", "node", &[])).unwrap()).changed());
    assert_eq!(fs::read(fixture.store.path()).unwrap(), bytes);
    assert!(!fixture.base.join("profile/.mcp.lock").exists());
}

#[test]
fn forged_tokens_and_counts_fail_before_profile_observation() {
    let fixture = Fixture::new();
    fixture.seed(b"invalid config to distinguish observation");
    let mut commands = vec![
        add("", "node", &[]),
        add("bad.name", "node", &[]),
        add(&"x".repeat(129), "node", &[]),
        add("ok", "", &[]),
        add("ok", "two words", &[]),
        add("ok", "tab\tword", &[]),
        add("ok", "line\nword", &[]),
        add("ok", "node", &[""]),
        add("ok", "node", &["two words"]),
        add("ok", "node", &["\u{85}"]),
        add("ok", &"x".repeat(4097), &[]),
        add("ok", "node", &[&"x".repeat(4097)]),
        McpCommand::Remove {
            server: "bad/name".into(),
        },
    ];
    commands.push(McpCommand::Add {
        server: "ok".into(),
        command: "node".into(),
        arguments: vec!["x".into(); 257],
    });
    for command in commands {
        assert_eq!(
            fixture.execute(command).unwrap_err(),
            NativeMcpManagementError::InvalidCommand
        );
    }
    assert!(!fixture.base.join("profile/.mcp.lock").exists());
}

#[test]
fn inclusive_command_limits_and_minimal_spelling_aggregate_are_enforced() {
    for command in [
        add(&"x".repeat(128), &"x".repeat(4096), &[&"x".repeat(4096)]),
        McpCommand::Add {
            server: "ok".into(),
            command: "node".into(),
            arguments: vec!["x".into(); 256],
        },
    ] {
        NativeMcpManagementService::validate_command(&command).unwrap();
    }
    // 5 grammar bytes + one-byte server/command + 31 full arguments + final arg.
    let mut arguments = vec!["x".repeat(4096); 31];
    arguments.push("x".repeat(MAX_MCP_COMMAND_BYTES - 7 - 31 * 4097 - 1));
    let mut command = McpCommand::Add {
        server: "s".into(),
        command: "c".into(),
        arguments,
    };
    NativeMcpManagementService::validate_command(&command).unwrap();
    let McpCommand::Add { arguments, .. } = &mut command else {
        unreachable!()
    };
    arguments.last_mut().unwrap().push('x');
    assert_eq!(
        NativeMcpManagementService::validate_command(&command),
        Err(NativeMcpManagementError::InvalidCommand)
    );
}

#[test]
fn parser_supported_commands_agree_with_direct_validation() {
    for input in [
        "",
        "list",
        "path",
        "remove Server_1",
        "add name node a;b $TOKEN '*.rs'",
        "add name node \u{a0}",
    ] {
        NativeMcpManagementService::validate_command(&input.parse().unwrap()).unwrap();
    }
}

#[test]
fn unsupported_variants_never_observe_or_create_profile() {
    let fixture = Fixture::new();
    for input in [
        "reload",
        "auth name",
        "auth name --open",
        "logout name",
        "resource list name",
        "resource templates name",
        "resource read name file:a",
        "resource complete name file:a arg",
        "prompt list name",
        "prompt get name prompt",
        "prompt complete name prompt arg",
    ] {
        assert_eq!(
            fixture.execute(input.parse().unwrap()).unwrap_err(),
            NativeMcpManagementError::RuntimeUnavailable
        );
        fixture.no_profile();
    }
}

#[test]
fn cancellation_prevents_observation_and_all_writes() {
    let fixture = Fixture::new();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    for command in [
        McpCommand::Summary,
        McpCommand::List,
        McpCommand::Path,
        add("name", "node", &[]),
        McpCommand::Remove {
            server: "name".into(),
        },
    ] {
        assert_eq!(
            fixture.service.execute(command, &cancellation).unwrap_err(),
            NativeMcpManagementError::Cancelled
        );
        fixture.no_profile();
    }
}

#[test]
fn unsafe_private_file_is_rejected_without_replacement() {
    let fixture = Fixture::new();
    let bytes = b"{}";
    fixture.seed(bytes);
    fs::set_permissions(fixture.store.path(), fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        fixture.execute(add("name", "node", &[])).unwrap_err(),
        NativeMcpManagementError::Store(NativeMcpConfigStoreError::UnsafePath)
    );
    assert_eq!(fs::read(fixture.store.path()).unwrap(), bytes);
    assert!(!fixture.base.join("profile/.mcp.lock").exists());
}

#[test]
fn diagnostic_forms_never_include_user_bytes() {
    let fixture = Fixture::new();
    assert_eq!(
        format!("{:?}", fixture.service),
        "NativeMcpManagementService { <redacted> }"
    );
    let receipt = fixture.execute(McpCommand::Path).unwrap();
    assert_eq!(
        format!("{receipt:?}"),
        "NativeMcpManagementReceipt { <redacted> }"
    );
    let error = fixture
        .execute(add("secret/name", "secret-command", &[]))
        .unwrap_err();
    assert_eq!(error.to_string(), "invalid MCP management command");
    assert_eq!(format!("{error:?}"), "InvalidCommand");
}
