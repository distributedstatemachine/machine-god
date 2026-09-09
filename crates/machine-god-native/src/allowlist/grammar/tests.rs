use super::*;

fn parsed(raw: &str) -> NativeAllowlistRequest {
    parse(raw, |tool| {
        matches!(
            tool,
            "read_file" | "web_fetch" | "MixedCase" | "install_skill"
        )
    })
    .unwrap()
}
fn target(raw: &str) -> (Scope, String, String) {
    match parsed(raw).command {
        Command::Mutate {
            scope,
            mutation:
                Mutation::Add {
                    permission,
                    pattern,
                }
                | Mutation::Remove {
                    permission,
                    pattern,
                },
        } => (scope, permission, pattern),
        _ => panic!("mutation target"),
    }
}

#[test]
fn views_scopes_and_verbs_are_ascii_case_insensitive_with_local_default() {
    for raw in ["", " \t", "view", "VIEW EFFECTIVE"] {
        assert_eq!(parsed(raw).command, Command::View(View::Effective));
    }
    assert_eq!(parsed("View LoCaL").command, Command::View(View::Local));
    assert_eq!(parsed("view USER").command, Command::View(View::User));
    assert_eq!(
        target("AdD CoMmAnD git *"),
        (Scope::Local, "bash".into(), "git *".into())
    );
    assert_eq!(
        target("USER ReMoVe URL https://example/*"),
        (Scope::User, "url".into(), "https://example/*".into())
    );
    for raw in [
        "user",
        "local view",
        "view user extra",
        "view\n",
        "add",
        "reset",
        "reset all extra",
        "reset commands\n",
        "add unknown x",
    ] {
        assert!(parse(raw, |_| true).is_err(), "{raw:?}");
    }
}

#[test]
fn every_reset_alias_preserves_exact_category_scope() {
    for (spellings, expected) in [
        (&["command", "commands"][..], Reset::Commands),
        (&["tool", "tools"][..], Reset::Tools),
        (&["url", "urls"][..], Reset::Urls),
        (
            &["web-fetch-domain", "web-fetch-domains"][..],
            Reset::WebFetchDomains,
        ),
        (&["all"][..], Reset::All),
    ] {
        for spelling in spellings {
            assert_eq!(
                parsed(&format!("USER RESET {}", spelling.to_ascii_uppercase())).command,
                Command::Mutate {
                    scope: Scope::User,
                    mutation: Mutation::Reset(expected)
                }
            );
        }
    }
}

#[test]
fn quoting_matches_first_close_or_open_rest_without_shell_escape_semantics() {
    for raw in [
        "add command \"git *\" ignored tail",
        "add command \"git *",
        "add command git *",
    ] {
        assert_eq!(target(raw).2, "git *");
    }
    assert_eq!(target("add command \"git \\\"tail\" ignored").2, "git \\");
    assert_eq!(target("add command 'git *'").2, "'git *'");
    assert_eq!(target("add command \" \t\r\n \"").2, "");
    assert_eq!(target("add command \" \r\ngit *\r\n \"").2, "git *");
    assert!(parse("add command \"\" tail", |_| true).is_err());
    assert!(parse("add command \"", |_| true).is_err());
}

#[test]
fn tool_registry_is_exact_with_historical_categories_and_web_fetch_exclusion() {
    assert_eq!(target("add tool read_file").1, "read");
    assert_eq!(target("add tool install_skill").1, "skill");
    assert_eq!(target("add tool MixedCase").1, "MixedCase");
    assert_eq!(target("add tool \"web_search\" ignored").1, "web_search");
    for raw in [
        "add tool READ_FILE",
        "add tool mixedcase",
        "add tool no_such_tool",
        "add tool web_fetch",
        "add tool web_search current news",
    ] {
        assert!(
            parse(raw, |name| matches!(
                name,
                "read_file" | "web_fetch" | "MixedCase"
            ))
            .is_err(),
            "{raw}"
        );
    }
    for name in [
        "edit",
        "create_folder",
        "open_file",
        "rename_file",
        "copy_file",
        "read",
        "list",
        "glob",
        "grep",
        "skill",
        "memory",
        "semantic_search",
        "web_search",
    ] {
        assert!(parse(&format!("add tool {name}"), |_| false).is_ok());
    }
    assert!(
        parsed("add tool MixedCase")
            .validate_registry(|_| false)
            .is_err()
    );
    assert!(parsed("add tool read").validate_registry(|_| false).is_ok());
}

#[test]
fn domains_preserve_pinned_dns_and_ipv6_oddities() {
    for (raw, canonical) in [
        ("EXAMPLE.COM.", "domain:example.com"),
        ("domain:EXAMPLE.com", "domain:example.com"),
        ("-host-.local", "domain:-host-.local"),
        ("[2001:db8::1]", "domain:[2001:db8::1]"),
        ("[::ffff:192.0.2.1].", "domain:[::ffff:192.0.2.1]"),
    ] {
        assert_eq!(target(&format!("add web-fetch-domain {raw}")).2, canonical);
    }
    for raw in [
        "[2001:DB8::1]",
        "[fe80::1%en0]",
        "https://example.com",
        "domain:example..",
        "example.com/path",
        "example:443",
        "*.example",
        "x?y",
        "DOMAIN:example.com",
        "a..b",
        ".",
        "é.com",
    ] {
        assert!(
            parse(&format!("add web-fetch-domain {raw}"), |_| false).is_err(),
            "{raw}"
        );
    }
}

#[test]
fn bounds_and_debug_never_disclose_input() {
    assert_eq!(
        parse(&"x".repeat(MAX_NATIVE_ALLOWLIST_REQUEST_BYTES + 1), |_| {
            false
        }),
        Err(Error::Limit)
    );
    let request = parsed("add command private-command-identity");
    assert!(!format!("{request:?}").contains("private-command-identity"));
    assert!(!format!("{:?}", request.command()).contains("private-command-identity"));
}

#[test]
fn owned_view_projection_filters_display_without_discarding_warning_evidence() {
    use crate::{NativeAllowlistSources, NativeConfiguredPermissionRules};
    use NativeConfiguredPermissionDecision::{Allow, Ask, Deny};
    let user = NativeConfiguredPermissionRules::new(vec![
        NativeConfiguredPermissionRule::new("read", "private-target", Allow).unwrap(),
        NativeConfiguredPermissionRule::new("read", "ask-target", Ask).unwrap(),
        NativeConfiguredPermissionRule::new("read", "deny-target", Deny).unwrap(),
        NativeConfiguredPermissionRule::new("web_fetch", "domain:example.com", Allow).unwrap(),
        NativeConfiguredPermissionRule::new("web_fetch", "https://invalid.example", Allow).unwrap(),
        NativeConfiguredPermissionRule::new("web_fetch", "invalid-ask", Ask).unwrap(),
        NativeConfiguredPermissionRule::new("web_fetch", "invalid-deny", Deny).unwrap(),
    ])
    .unwrap();
    let sources = NativeAllowlistSources {
        user,
        local: Some(NativeConfiguredPermissionRules::default()),
    };
    assert_eq!(sources.display_rules(View::User).count(), 2);
    assert_eq!(sources.user().rules().len(), 7);
    assert_eq!(sources.user().web_fetch_warning_count(), 3);
    assert!(sources.display_rules(View::Local).next().is_none());
    assert!(sources.display_rules(View::Effective).next().is_none());
    assert!(sources.user_shadowed_by_local());
    assert!(!format!("{sources:?}").contains("private-target"));
}
