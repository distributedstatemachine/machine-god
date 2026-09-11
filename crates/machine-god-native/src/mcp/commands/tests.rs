use super::*;

#[test]
fn routes_management_commands_and_explicit_browser_confirmation() {
    for (input, expected) in [
        (" \t", McpCommand::Summary),
        (" list ", McpCommand::List),
        ("path", McpCommand::Path),
        ("reload", McpCommand::Reload),
        (
            "remove app",
            McpCommand::Remove {
                server: "app".into(),
            },
        ),
        (
            "logout app",
            McpCommand::Logout {
                server: "app".into(),
            },
        ),
        (
            "auth app",
            McpCommand::Authenticate {
                server: "app".into(),
                open_browser: false,
            },
        ),
        (
            "auth app --open",
            McpCommand::Authenticate {
                server: "app".into(),
                open_browser: true,
            },
        ),
    ] {
        assert_eq!(input.parse(), Ok(expected), "{input}");
    }
}

#[test]
fn add_preserves_pinned_tokenization_without_shell_interpretation() {
    assert_eq!(
        "add app node script.js 'two words' $(secret)".parse(),
        Ok(McpCommand::Add {
            server: "app".into(),
            command: "node".into(),
            arguments: ["script.js", "'two", "words'", "$(secret)"]
                .map(str::to_owned)
                .into(),
        })
    );
}

#[test]
fn routes_all_seven_feature_actions_and_retains_exact_remaining_values() {
    for (input, expected) in [
        (
            "resource list app",
            McpFeatureCommand::ResourceList {
                server: "app".into(),
            },
        ),
        (
            "resource templates app",
            McpFeatureCommand::ResourceTemplates {
                server: "app".into(),
            },
        ),
        (
            "resource read app custom://日本語/a b",
            McpFeatureCommand::ResourceRead {
                server: "app".into(),
                uri: "custom://日本語/a b".into(),
            },
        ),
        (
            "resource complete app custom://{key} key prefix with spaces",
            McpFeatureCommand::ResourceComplete {
                server: "app".into(),
                uri_template: "custom://{key}".into(),
                argument: "key".into(),
                value: "prefix with spaces".into(),
            },
        ),
        (
            "prompt list app",
            McpFeatureCommand::PromptList {
                server: "app".into(),
            },
        ),
        (
            "prompt get app review",
            McpFeatureCommand::PromptGet {
                server: "app".into(),
                prompt: "review".into(),
                arguments: BTreeMap::new(),
            },
        ),
        (
            "prompt complete app review tone",
            McpFeatureCommand::PromptComplete {
                server: "app".into(),
                prompt: "review".into(),
                argument: "tone".into(),
                value: String::new(),
            },
        ),
    ] {
        assert_eq!(input.parse(), Ok(McpCommand::Feature(expected)), "{input}");
    }
    let parsed = "prompt get app review {\"tone\":\"brief\\nexact\"}"
        .parse::<McpCommand>()
        .unwrap();
    let McpCommand::Feature(McpFeatureCommand::PromptGet { arguments, .. }) = parsed else {
        panic!("wrong action")
    };
    assert_eq!(arguments["tone"], "brief\nexact");
}

#[test]
fn rejects_extra_missing_unknown_and_control_input() {
    for input in [
        "/mcp list",
        "List",
        "list app",
        "path extra",
        "reload extra",
        "add app",
        "remove",
        "remove app extra",
        "auth app --yes",
        "auth app --open extra",
        "logout app extra",
        "resource",
        "resource list",
        "resource read app",
        "resource list app extra",
        "resource complete app custom://{key}",
        "prompt get app",
        "prompt complete app review",
        "resource get app key",
        "prompt read app key",
        "auth app\n",
        "list\r",
        "list\0",
        "remove ápp",
        "remove app.name",
        "remove app\u{1b}",
    ] {
        assert!(input.parse::<McpCommand>().is_err(), "{input:?}");
    }
}

#[test]
fn prompt_arguments_reject_non_strings_duplicates_and_trailing_json() {
    for arguments in [
        "[]",
        "null",
        "{\"x\":1}",
        "{\"x\":{}}",
        "{\"x\":\"a\",\"x\":\"b\"}",
        "{\"\":\"a\"}",
        "{} {}",
    ] {
        assert!(
            format!("prompt get app review {arguments}")
                .parse::<McpCommand>()
                .is_err()
        );
    }
    let deep = format!(
        "prompt get app review {{\"x\":{}0{}}}",
        "[".repeat(20_000),
        "]".repeat(20_000)
    );
    assert!(deep.parse::<McpCommand>().is_err());
    let mut map = BTreeMap::new();
    for i in 0..MAX_PROMPT_ARGUMENTS {
        map.insert(format!("key{i}"), "value");
    }
    let parse = |map: &BTreeMap<String, &str>| {
        format!(
            "prompt get app review {}",
            serde_json::to_string(map).unwrap()
        )
        .parse::<McpCommand>()
    };
    assert!(parse(&map).is_ok());
    map.insert("overflow".into(), "value");
    assert!(parse(&map).is_err());
}

#[test]
fn byte_and_count_limits_are_inclusive() {
    let alias = "a".repeat(MAX_SERVER_BYTES);
    assert!(format!("remove {alias}").parse::<McpCommand>().is_ok());
    assert_eq!(
        format!("remove {alias}a")
            .parse::<McpCommand>()
            .unwrap_err()
            .kind(),
        McpCommandParseErrorKind::ResourceLimit
    );
    let uri = "é".repeat(MAX_URI_BYTES / 2);
    assert!(
        format!("resource read app {uri}")
            .parse::<McpCommand>()
            .is_ok()
    );
    assert!(
        format!("resource read app {uri}x")
            .parse::<McpCommand>()
            .is_err()
    );
    let arguments = " x".repeat(MAX_ARGUMENTS);
    assert!(
        format!("add app command{arguments}")
            .parse::<McpCommand>()
            .is_ok()
    );
    assert!(
        format!("add app command{arguments} x")
            .parse::<McpCommand>()
            .is_err()
    );
    let input = " ".repeat(MAX_MCP_COMMAND_BYTES);
    assert_eq!(input.parse(), Ok(McpCommand::Summary));
    assert_eq!(format!("{input} ").parse::<McpCommand>(), Err(limit()));
}

#[test]
fn debug_and_errors_never_echo_payloads() {
    for input in [
        "add secret-server secret-command secret-value",
        "auth secret-server --open",
        "resource read secret-server secret-uri",
        "prompt get secret-server secret-prompt {\"secret-key\":\"secret-value\"}",
    ] {
        let parsed: McpCommand = input.parse().unwrap();
        assert!(!format!("{parsed:?}").contains("secret"));
        if let McpCommand::Feature(feature) = parsed {
            assert!(!format!("{feature:?}").contains("secret"));
        }
    }
    let error = "secret-invalid".parse::<McpCommand>().unwrap_err();
    assert!(!format!("{error} {error:?}").contains("secret"));
}
