use super::*;

fn decode(value: serde_json::Value) -> Result<McpConfig, McpConfigError> {
    let bytes = serde_json::to_vec(&value).unwrap();
    // Release the producer fixture before exercising the codec's own allocations.
    drop(value);
    McpConfig::decode(&bytes)
}

fn stdio(config: &McpServerConfig) -> &McpStdioConfig {
    let McpTransportConfig::Stdio(value) = config.transport() else {
        panic!("stdio expected")
    };
    value
}

#[test]
fn producer_string_vector_alias_defaults_and_precedence() {
    // Derived from pinned builtins/mcp.zig's string-command/environment tests.
    let config = McpConfig::decode(br#"{"mcp":{"z":{"command":"node","args":["server.js",""]},"a":{"type":"local","command":["python","-m","server"],"args":false,"environment":{"TOKEN":"selected"},"env":{"TOKEN":"ignored"},"required":true}}}"#).unwrap();
    assert_eq!(
        config
            .servers()
            .iter()
            .map(McpServerConfig::name)
            .collect::<Vec<_>>(),
        ["z", "a"]
    );
    let z = config.server("z").unwrap();
    assert_eq!(stdio(z).command(), "node");
    assert_eq!(&*stdio(z).args()[1], "");
    assert_eq!(z.startup_timeout_ms(), 10_000);
    assert_eq!(z.operation_timeout_ms(), 60_000);
    assert_eq!(stdio(z).restart_limit(), 1);
    assert!(z.enabled());
    assert!(!z.required());
    let a = config.server("a").unwrap();
    assert_eq!(
        stdio(a)
            .args()
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<&str>>(),
        ["-m", "server"]
    );
    assert_eq!(&*a.environment()["TOKEN"], "selected");
    assert!(a.required());
    let encoded = config.encode().unwrap();
    assert_eq!(McpConfig::decode(&encoded).unwrap(), config);
    assert_eq!(
        McpConfig::decode(&encoded).unwrap().encode().unwrap(),
        encoded
    );
}

#[test]
fn producer_remote_oauth_and_header_environment_example() {
    // Pinned builtins/mcp.zig's HTTP credential configuration producer fixture.
    let config = McpConfig::decode(br#"{"mcp":{"api":{"type":"http","url":"https://api.example.com/mcp","headers":{"X-Identity":"one"},"header_env":{"X-Workspace":"MCP_WORKSPACE"},"bearer_token_env":"MCP_TOKEN","oauth":{"resource":"https://api.example.com/mcp","issuer":"https://login.example.com","client_id":"fx-client","client_secret_env":"MCP_CLIENT_SECRET","client_metadata_url":"https://client.example/fx.json","scopes":["tools.read","tools.call"]},"startup_timeout_ms":2500,"operation_timeout_ms":5000}}}"#).unwrap();
    let server = config.server("api").unwrap();
    let McpTransportConfig::Http(remote) = server.transport() else {
        panic!("HTTP expected")
    };
    assert_eq!(remote.url(), "https://api.example.com/mcp");
    assert_eq!(&*remote.headers()["X-Identity"], "one");
    assert_eq!(&*remote.header_env()["X-Workspace"], "MCP_WORKSPACE");
    assert_eq!(remote.bearer_token_env(), Some("MCP_TOKEN"));
    let oauth = remote.oauth().unwrap();
    assert_eq!(oauth.client_id(), Some("fx-client"));
    assert_eq!(oauth.resource(), Some("https://api.example.com/mcp"));
    assert_eq!(oauth.issuer(), Some("https://login.example.com"));
    assert_eq!(oauth.client_secret_env(), Some("MCP_CLIENT_SECRET"));
    assert_eq!(
        oauth.client_metadata_url(),
        Some("https://client.example/fx.json")
    );
    assert_eq!(oauth.scopes().len(), 2);
    assert_eq!(server.startup_timeout_ms(), 2500);
    assert_eq!(server.operation_timeout_ms(), 5000);
    assert_eq!(
        McpConfig::decode(&config.encode().unwrap()).unwrap(),
        config
    );
}

#[test]
fn producer_sse_and_explicit_loopback_ports() {
    for url in [
        "https://mcp.example.com/rpc?workspace=one",
        "http://localhost:4321/mcp",
        "http://127.0.0.1:80/mcp",
        "http://[::1]:4321/mcp",
    ] {
        let config =
            decode(serde_json::json!({"mcp":{"remote":{"type":"sse","url":url}}})).unwrap();
        assert!(matches!(
            config.server("remote").unwrap().transport(),
            McpTransportConfig::Sse(_)
        ));
    }
}

#[test]
fn producer_lifecycle_integer_bounds() {
    let config = McpServerConfig::decode("policy", br#"{"command":["node"],"enabled":false,"startup_timeout_ms":4294967295,"operation_timeout_ms":1,"restart_limit":255}"#).unwrap();
    assert_eq!(config.startup_timeout_ms(), u32::MAX);
    assert_eq!(config.operation_timeout_ms(), 1);
    assert_eq!(stdio(&config).restart_limit(), 255);
    assert!(!config.enabled());
    for (key, value) in [
        ("startup_timeout_ms", serde_json::json!(0)),
        ("operation_timeout_ms", serde_json::json!(4_294_967_296_u64)),
        ("restart_limit", serde_json::json!(256)),
        ("restart_limit", serde_json::json!(-1)),
        ("enabled", serde_json::json!(1)),
        ("required", serde_json::json!("yes")),
        ("startup_timeout_ms", serde_json::json!(1.0)),
    ] {
        let mut server = serde_json::json!({"command":"node"});
        server[key] = value;
        assert_eq!(
            decode(serde_json::json!({"mcp":{"bad":server}})),
            Err(McpConfigError::Invalid)
        );
    }
}

#[test]
fn empty_profile_and_environment_alias() {
    assert!(McpConfig::decode(b"{}").unwrap().servers().is_empty());
    assert_eq!(McpConfig::new().encode().unwrap(), br#"{"mcp":{}}"#);
    let server = McpServerConfig::decode(
        "env",
        br#"{"command":"node","env":{"EMPTY":"","TOKEN":"value"}}"#,
    )
    .unwrap();
    assert_eq!(&*server.environment()["TOKEN"], "value");
    assert_eq!(&*server.environment()["EMPTY"], "");
}

#[test]
fn duplicate_keys_including_escaped_aliases_are_rejected() {
    for input in [
        r#"{"mcp":{},"mcp":{}}"#,
        r#"{"mcp":{"same":{"command":"a"},"same":{"command":"b"}}}"#,
        r#"{"mcp":{"a":{"command":"x","command":"y"}}}"#,
        r#"{"mcp":{"a":{"command":"x","env":{"TOKEN":"a","TOKEN":"b"}}}}"#,
    ] {
        assert_eq!(
            McpConfig::decode(input.as_bytes()),
            Err(McpConfigError::Invalid)
        );
    }
}

#[test]
fn malformed_unknown_inactive_and_unusable_commands_rejected() {
    for input in [
        "[]",
        "null",
        "{} {}",
        r#"{"mcp":[]}"#,
        r#"{"unknown":1}"#,
        r#"{"mcp":{"a":{}}}"#,
        r#"{"mcp":{"a":{"command":[]}}}"#,
        r#"{"mcp":{"a":{"command":""}}}"#,
        r#"{"mcp":{"a":{"command":["node",1]}}}"#,
        r#"{"mcp":{"a":{"command":"node","url":"https://example.com"}}}"#,
        r#"{"mcp":{"a":{"type":"http","url":"https://example.com","command":"node"}}}"#,
        r#"{"mcp":{"a":{"command":"node","typo":true}}}"#,
        r#"{"mcp":{"a":{"command":"node","type":"unknown"}}}"#,
    ] {
        assert_eq!(
            McpConfig::decode(input.as_bytes()),
            Err(McpConfigError::Invalid),
            "{input}"
        );
    }
}

#[test]
fn invalid_names_and_control_strings_rejected() {
    for name in ["", "a b", "a/b", "a.b", "ümlaut", "a\n"] {
        assert_eq!(
            McpServerConfig::stdio(name, "node", &[]),
            Err(McpConfigError::Invalid)
        );
    }
    assert!(McpServerConfig::stdio(&"a".repeat(128), "node", &[]).is_ok());
    assert_eq!(
        McpServerConfig::stdio(&"a".repeat(129), "node", &[]),
        Err(McpConfigError::Limit)
    );
    for command in ["node\0", "node\n", "node\u{85}"] {
        assert_eq!(
            McpServerConfig::stdio("a", command, &[]),
            Err(McpConfigError::Invalid)
        );
    }
    for env in [
        serde_json::json!({"1TOKEN":"x"}),
        serde_json::json!({"TOKEN":"a\n"}),
        serde_json::json!({"TOKEN":false}),
        serde_json::json!({"A=B":"x"}),
    ] {
        assert_eq!(
            decode(serde_json::json!({"mcp":{"a":{"command":"node","env":env}}})),
            Err(McpConfigError::Invalid)
        );
    }
}

#[test]
fn reserved_ambiguous_and_injected_headers_rejected() {
    for key in [
        "Authorization",
        "HOST",
        "mcp-param-argument",
        "MCP-Protocol-Version",
        "content-length",
        "bad name",
        "",
    ] {
        assert_eq!(
            decode(
                serde_json::json!({"mcp":{"a":{"type":"http","url":"https://example.com","headers":{key:"x"}}}})
            ),
            Err(McpConfigError::Invalid)
        );
    }
    for fields in [
        serde_json::json!({"headers":{"X-A":"x","x-a":"y"}}),
        serde_json::json!({"headers":{"X-A":"x"},"header_env":{"x-a":"TOKEN"}}),
        serde_json::json!({"headers":{"X-A":"injected\r\nHost:x"}}),
        serde_json::json!({"header_env":{"X-A":"BAD-NAME"}}),
        serde_json::json!({"bearer_token_env":"BAD-NAME"}),
    ] {
        let mut server = serde_json::json!({"type":"http","url":"https://example.com"});
        server
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        assert_eq!(
            decode(serde_json::json!({"mcp":{"a":server}})),
            Err(McpConfigError::Invalid)
        );
    }
}

#[test]
fn producer_insecure_urls_and_oauth_errors_rejected() {
    for url in [
        "http://example.com/mcp",
        "http://localhost/mcp",
        "https://user@example.com/mcp",
        "https://example.com/mcp#fragment",
        "https://",
        "file:///tmp/mcp",
        "https://example.com/\n",
    ] {
        assert_eq!(
            decode(serde_json::json!({"mcp":{"a":{"type":"http","url":url}}})),
            Err(McpConfigError::Invalid)
        );
    }
    for oauth in [
        serde_json::json!({"client_secret_env":"SECRET"}),
        serde_json::json!({"client_id":"a","client_secret_env":"BAD-NAME"}),
        serde_json::json!({"client_metadata_url":"http://127.0.0.1:4321/client.json"}),
        serde_json::json!({"client_metadata_url":"https://client.example"}),
        serde_json::json!({"client_metadata_url":"https://client.example/"}),
        serde_json::json!({"scopes":[false]}),
        serde_json::json!({"unknown":true}),
    ] {
        assert_eq!(
            decode(
                serde_json::json!({"mcp":{"a":{"type":"http","url":"https://example.com","oauth":oauth}}})
            ),
            Err(McpConfigError::Invalid)
        );
    }
}

#[test]
fn constructor_preserves_exact_arguments_without_shell_interpretation() {
    let server =
        McpServerConfig::stdio("local", "node", &["$(never)", "a b", "", "$TOKEN"]).unwrap();
    assert_eq!(
        stdio(&server)
            .args()
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<&str>>(),
        ["$(never)", "a b", "", "$TOKEN"]
    );
}

#[test]
fn mutation_preserves_order_requires_explicit_replace_and_returns_old_values() {
    let mut config = McpConfig::new();
    config
        .insert(McpServerConfig::stdio("z", "old", &[]).unwrap())
        .unwrap();
    config
        .insert(McpServerConfig::stdio("a", "node", &[]).unwrap())
        .unwrap();
    let before = config.clone();
    assert_eq!(
        config.insert(McpServerConfig::stdio("z", "new", &[]).unwrap()),
        Err(McpConfigError::AlreadyExists)
    );
    assert_eq!(config, before);
    let old = config
        .replace(McpServerConfig::stdio("z", "new", &[]).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(stdio(&old).command(), "old");
    assert_eq!(config.servers()[0].name(), "z");
    assert!(config.remove("unknown").is_none());
    assert_eq!(stdio(&config.remove("z").unwrap()).command(), "new");
    assert_eq!(config.servers()[0].name(), "a");
}

#[test]
fn server_limit_is_atomic_for_insert_but_allows_replacement() {
    let mut config = McpConfig::new();
    for i in 0..MAX_SERVERS {
        config
            .insert(McpServerConfig::stdio(&format!("s{i}"), "node", &[]).unwrap())
            .unwrap();
    }
    let before = config.clone();
    assert_eq!(
        config.insert(McpServerConfig::stdio("extra", "node", &[]).unwrap()),
        Err(McpConfigError::Limit)
    );
    assert_eq!(config, before);
    assert!(
        config
            .replace(McpServerConfig::stdio("s0", "new", &[]).unwrap())
            .unwrap()
            .is_some()
    );
}

#[test]
fn aggregate_replacement_limit_keeps_old_config_unchanged() {
    let arg = "x".repeat(16 * 1024);
    let large = vec![arg.as_str(); 20];
    let mut config = McpConfig::new();
    config
        .insert(McpServerConfig::stdio("a", "node", &large).unwrap())
        .unwrap();
    config
        .insert(McpServerConfig::stdio("b", "node", &[]).unwrap())
        .unwrap();
    let before = config.clone();
    assert_eq!(
        config.replace(McpServerConfig::stdio("b", "node", &large).unwrap()),
        Err(McpConfigError::Limit)
    );
    assert_eq!(config, before);
}

#[test]
fn encoded_escaping_limit_is_bounded_and_atomic() {
    let quote = "\"".repeat(16 * 1024);
    // The decoded string budget is reached before this can escape past 1 MiB.
    assert_eq!(
        McpServerConfig::stdio("a", "node", &vec![quote.as_str(); 32]),
        Err(McpConfigError::Limit)
    );
    assert_eq!(
        McpConfig::decode(&vec![b' '; MAX_CONFIG_BYTES + 1]),
        Err(McpConfigError::Limit)
    );
    assert_eq!(
        super::json::encode(&"x".repeat(MAX_CONFIG_BYTES)),
        Err(McpConfigError::Limit)
    );
}

#[test]
fn depth_nodes_and_string_limits_precede_semantic_admission() {
    let deep = format!("{}0{}", "[".repeat(10), "]".repeat(10));
    assert_eq!(
        McpConfig::decode(deep.as_bytes()),
        Err(McpConfigError::Limit)
    );
    let nodes = format!("[{}]", vec!["0"; 16_384].join(","));
    assert_eq!(
        McpConfig::decode(nodes.as_bytes()),
        Err(McpConfigError::Limit)
    );
    let huge = serde_json::to_vec(&"x".repeat(16 * 1024 + 1)).unwrap();
    assert_eq!(McpConfig::decode(&huge), Err(McpConfigError::Limit));
    let total = serde_json::to_vec(&vec!["x".repeat(16 * 1024); 33]).unwrap();
    assert_eq!(McpConfig::decode(&total), Err(McpConfigError::Limit));
}

#[test]
fn collection_and_command_bounds() {
    assert!(McpServerConfig::stdio("a", &"x".repeat(4096), &vec![""; 256]).is_ok());
    assert_eq!(
        McpServerConfig::stdio("a", &"x".repeat(4097), &[]),
        Err(McpConfigError::Limit)
    );
    assert_eq!(
        McpServerConfig::stdio("a", "node", &vec![""; 257]),
        Err(McpConfigError::Limit)
    );
    let env: serde_json::Map<_, _> = (0..257)
        .map(|i| (format!("E{i}"), serde_json::json!("")))
        .collect();
    assert_eq!(
        decode(serde_json::json!({"mcp":{"a":{"command":"node","env":env}}})),
        Err(McpConfigError::Limit)
    );
    assert_eq!(
        decode(
            serde_json::json!({"mcp":{"a":{"type":"http","url":"https://example.com","oauth":{"scopes":vec!["";65]}}}})
        ),
        Err(McpConfigError::Limit)
    );
}

#[test]
fn debug_and_errors_do_not_disclose_input() {
    let server = McpServerConfig::decode("secret_alias", br#"{"command":"secret_command","args":["secret_argument"],"env":{"SECRET_ENV":"secret_value"}}"#).unwrap();
    let text = format!("{server:?} {:?}", server.transport());
    for secret in [
        "secret_alias",
        "secret_command",
        "secret_argument",
        "SECRET_ENV",
        "secret_value",
    ] {
        assert!(!text.contains(secret));
    }
    let remote = McpServerConfig::decode("secret", br#"{"type":"http","url":"https://secret.example","headers":{"X-Secret":"secret_value"},"oauth":{"client_id":"secret_client"}}"#).unwrap();
    let McpTransportConfig::Http(config) = remote.transport() else {
        panic!("HTTP expected")
    };
    let text = format!("{remote:?} {config:?} {:?}", config.oauth());
    assert!(!text.contains("secret"));
    let error = McpServerConfig::decode("secret", br#"{"command":"secret\n"}"#).unwrap_err();
    assert!(!format!("{error:?} {error}").contains("secret"));
}

#[test]
fn header_server_and_optional_field_boundaries() {
    let mut headers: serde_json::Map<_, _> = (0..128)
        .map(|i| (format!("X-H{i}"), serde_json::json!("")))
        .collect();
    let profile = |headers| serde_json::json!({"mcp":{"a":{"type":"http","url":"https://example.com","headers":headers,"bearer_token_env":null,"oauth":{"client_id":null,"resource":null}}}});
    let config = decode(profile(headers.clone())).unwrap();
    let McpTransportConfig::Http(remote) = config.server("a").unwrap().transport() else {
        panic!("HTTP expected")
    };
    assert_eq!(remote.bearer_token_env(), None);
    assert_eq!(remote.oauth().unwrap().client_id(), None);
    headers.insert("X-Extra".into(), serde_json::json!(""));
    assert_eq!(decode(profile(headers)), Err(McpConfigError::Limit));
    let servers: serde_json::Map<_, _> = (0..65)
        .map(|i| (format!("s{i}"), serde_json::json!({"command":"node"})))
        .collect();
    assert_eq!(
        decode(serde_json::json!({"mcp":servers})),
        Err(McpConfigError::Limit)
    );
    let server =
        McpServerConfig::decode("zero", br#"{"command":"node","restart_limit":0}"#).unwrap();
    assert_eq!(stdio(&server).restart_limit(), 0);
}

#[test]
fn canonical_bytes_and_typed_strings_have_no_excess_capacity() {
    let server = McpServerConfig::stdio("a", "node", &["a", "b"]).unwrap();
    let mut config = McpConfig::new();
    config.insert(server).unwrap();
    let bytes = config.encode().unwrap();
    assert_eq!(bytes.capacity(), bytes.len());
    // Box-backed public strings/slices cannot retain caller-supplied capacities.
    assert_eq!(stdio(config.server("a").unwrap()).args().len(), 2);
}

#[test]
fn metadata_query_cannot_supply_missing_path() {
    assert_eq!(
        decode(
            serde_json::json!({"mcp":{"a":{"type":"http","url":"https://example.com","oauth":{"client_metadata_url":"https://client.example?next=/metadata.json"}}}})
        ),
        Err(McpConfigError::Invalid)
    );
}

#[test]
fn oauth_url_fields_obey_the_url_bound_before_ownership() {
    for key in ["resource", "issuer", "client_metadata_url"] {
        let url = format!("https://example.com/{}", "x".repeat(4096));
        assert_eq!(
            decode(
                serde_json::json!({"mcp":{"a":{"type":"http","url":"https://example.com","oauth":{key:url}}}})
            ),
            Err(McpConfigError::Limit)
        );
    }
}
