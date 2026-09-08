use machine_god_native::{
    MAX_CONFIG_BYTES, MAX_CONFIGURED_PERMISSION_TARGET_BYTES, MAX_TERMINAL_CWD_BYTES,
    NativeConfiguredPermissionDecision as Decision, NativeConfiguredPermissionError as Error,
    NativeConfiguredPermissionRule as Rule, NativeConfiguredPermissionRules as Rules,
    NativePermissionTargetKind as Kind, NativePreparedPermissionTarget as Target,
};

fn rules(items: &[(&str, &str, Decision)]) -> Rules {
    Rules::new(
        items
            .iter()
            .map(|(permission, pattern, decision)| {
                Rule::new(permission, pattern, *decision).unwrap()
            })
            .collect(),
    )
    .unwrap()
}

fn decide(rules: &Rules, tool: &str, target: &str, kind: Kind) -> Option<Decision> {
    rules
        .decide(&Target::new("/workspace", tool, target, kind).unwrap())
        .unwrap()
}

#[test]
fn ordered_matching_is_last_wins_not_deny_always_wins() {
    let policy = rules(&[
        ("*", "*", Decision::Deny),
        ("edit", "src/**", Decision::Allow),
        ("write_file", "src/private/*", Decision::Ask),
        ("edit", "src/private/allowed", Decision::Allow),
    ]);
    for (path, expected) in [
        ("outside", Decision::Deny),
        ("src", Decision::Allow),
        ("src/a", Decision::Allow),
        ("src/private/a", Decision::Ask),
        ("src/private/allowed", Decision::Allow),
    ] {
        assert_eq!(
            decide(&policy, "write_file", path, Kind::PathCreateParent),
            Some(expected)
        );
    }
    assert_eq!(
        decide(&Rules::default(), "write_file", "a", Kind::PathCreateParent),
        None
    );
}

#[test]
fn aliases_match_categories_and_actual_tool_names_without_fabricated_aliases() {
    for (tool, alias) in [
        ("read_file", "read"),
        ("write_file", "edit"),
        ("edit_file", "edit"),
        ("list_files", "list"),
        ("glob_files", "glob"),
        ("grep_files", "grep"),
        ("run_command", "bash"),
        ("skill", "skill"),
        ("install_skill", "skill"),
        ("delete_file", "delete_file"),
        ("rename_file", "rename_file"),
        ("copy_file", "copy_file"),
        ("file_info", "file_info"),
        ("web_search", "web_search"),
        ("terminal", "terminal"),
    ] {
        assert_eq!(
            decide(
                &rules(&[(alias, "*", Decision::Allow)]),
                tool,
                "value",
                Kind::None
            ),
            Some(Decision::Allow)
        );
        assert_eq!(
            decide(
                &rules(&[(tool, "*", Decision::Deny)]),
                tool,
                "value",
                Kind::None
            ),
            Some(Decision::Deny)
        );
    }
    assert_eq!(
        decide(
            &rules(&[("bash", "*", Decision::Allow)]),
            "terminal",
            "exec",
            Kind::None
        ),
        None
    );
    assert_eq!(
        decide(
            &rules(&[("read", "*", Decision::Allow)]),
            "file_info",
            "a",
            Kind::None
        ),
        None
    );
    assert_eq!(
        decide(
            &rules(&[("*_file", "*", Decision::Ask)]),
            "read_file",
            "a",
            Kind::None
        ),
        Some(Decision::Ask)
    );
}

#[test]
fn patterns_are_byte_wildcards_not_globs_regex_or_unicode_scalars() {
    for (pattern, candidate, matches) in [
        ("?", "é", false),
        ("??", "é", true),
        ("?", "a", true),
        ("*", "a/b\nc", true),
        ("a?c", "a/c", true),
        ("[ab]", "a", false),
        ("[ab]", "[ab]", true),
        ("a\\*", "a\\path", true),
        ("a\\*", "a*", false),
        ("*a", "*xa", true),
        ("a*b*c", "axybzzc", true),
        ("a*b*c", "axybzzd", false),
        ("***", "", true),
        ("", "", true),
        ("", "a", false),
        ("É", "é", false),
    ] {
        let policy = rules(&[("read", pattern, Decision::Allow)]);
        assert_eq!(
            decide(&policy, "read_file", candidate, Kind::None).is_some(),
            matches,
            "{pattern:?} / {candidate:?}"
        );
    }
}

#[test]
fn directory_tree_special_case_includes_root_and_respects_boundaries() {
    for (pattern, candidate, matches) in [
        ("src/**", "src", true),
        ("src/**", "src/a/b", true),
        ("src/**", "src/", true),
        ("src/**", "src-other", false),
        ("/**", "/", true),
        ("/**", "/a", true),
        ("/**", "", false),
        ("/**", "relative", false),
        ("s*/**", "src", false),
        ("s*/**", "src/a", true),
        ("s*/**", "s*", true),
    ] {
        assert_eq!(
            decide(
                &rules(&[("read", pattern, Decision::Allow)]),
                "read_file",
                candidate,
                Kind::None
            )
            .is_some(),
            matches
        );
    }
}

#[test]
fn prepared_path_kind_controls_relative_presentation_without_io() {
    let policy = rules(&[("*", "src/a", Decision::Allow), ("*", ".", Decision::Ask)]);
    for kind in [
        Kind::PathExisting,
        Kind::PathOptionalExisting,
        Kind::PathCreateParent,
        Kind::PathExistingParent,
    ] {
        assert_eq!(
            decide(&policy, "read_file", "/workspace/src/a", kind),
            Some(Decision::Allow)
        );
        assert_eq!(
            decide(&policy, "read_file", "/workspace", kind),
            Some(Decision::Ask)
        );
        assert_eq!(
            decide(&policy, "read_file", "/workspace-other/src/a", kind),
            None
        );
        assert_eq!(
            decide(&policy, "read_file", "src/a", kind),
            Some(Decision::Allow)
        );
    }
    for kind in [Kind::None, Kind::Url] {
        assert_eq!(decide(&policy, "read_file", "/workspace/src/a", kind), None);
        for tool in ["copy_file", "rename_file"] {
            assert_eq!(
                decide(&policy, tool, "/workspace/src/a", kind),
                Some(Decision::Allow)
            );
        }
    }
    assert_eq!(
        policy
            .decide(
                &Target::new(
                    "/workspace/",
                    "read_file",
                    "/workspace/src/a",
                    Kind::PathExisting
                )
                .unwrap()
            )
            .unwrap(),
        Some(Decision::Allow)
    );
    assert_eq!(
        policy
            .decide(&Target::new("/", "read_file", "/src/a", Kind::PathExisting).unwrap())
            .unwrap(),
        Some(Decision::Allow)
    );
}

#[test]
fn command_matching_strips_cwd_and_length_framed_environment_only_for_patterns() {
    let policy = rules(&[
        ("bash", "printf 'a::b'", Decision::Allow),
        ("sandbox", "printf 'a::b'", Decision::Allow),
    ]);
    for identity in [
        "printf 'a::b'",
        "@fx-terminal-env:clean:8:/bin/zsh::printf 'a::b'",
        "@fx-terminal-env:user:12:/opt/bin::sh::printf 'a::b'",
        "@fx-terminal-env:clean:+8:/bin/zsh::printf 'a::b'",
        "@fx-terminal-env:clean:0_8:/bin/zsh::printf 'a::b'",
    ] {
        // Bare plain commands containing '::' are not prepared bash targets:
        // legacy command targets include cwd before their first separator.
        let target = format!("/workspace::{identity}");
        assert_eq!(
            decide(&policy, "run_command", &target, Kind::CommandCwd),
            Some(Decision::Allow)
        );
        assert_eq!(
            decide(&policy, "sandbox", identity, Kind::None),
            Some(Decision::Allow)
        );
        if identity.starts_with('@') {
            assert_eq!(
                decide(&policy, "run_command", identity, Kind::CommandCwd),
                Some(Decision::Allow)
            );
        }
    }
    for identity in [
        "@fx-terminal-env:clean:999999999999999999999999999:/bin/zsh::printf 'a::b'",
        "@fx-terminal-env:clean:99:/bin/zsh::printf 'a::b'",
        "@fx-terminal-env:clean:8:/bin/zsh:printf 'a::b'",
        "@fx-terminal-env:clean:x:/bin/zsh::printf 'a::b'",
    ] {
        assert_eq!(
            decide(&policy, "run_command", identity, Kind::CommandCwd),
            None
        );
    }
    let target = Target::new(
        "/workspace",
        "run_command",
        "/workspace::@fx-terminal-env:clean:8:/bin/zsh::printf 'a::b'",
        Kind::CommandCwd,
    )
    .unwrap();
    assert!(target.target().contains("@fx-terminal-env"));
    assert_eq!(target.workspace_root(), "/workspace");
    assert_eq!(target.tool_name(), "run_command");
    assert_eq!(target.kind(), Kind::CommandCwd);
}

#[test]
fn command_cwd_display_is_not_bash_projection_for_other_categories() {
    let policy = rules(&[
        ("custom", "src::echo ok", Decision::Allow),
        ("custom", ".::echo ok", Decision::Ask),
    ]);
    assert_eq!(
        decide(
            &policy,
            "custom",
            "/workspace/src::echo ok",
            Kind::CommandCwd
        ),
        Some(Decision::Allow)
    );
    assert_eq!(
        decide(
            &policy,
            "custom",
            "/workspace::@fx-terminal-env:clean:8:/bin/zsh::echo ok",
            Kind::CommandCwd
        ),
        Some(Decision::Ask)
    );
    assert_eq!(
        decide(&policy, "custom", "/workspace/src::echo ok", Kind::None),
        None
    );
    assert_eq!(
        policy
            .decide(&Target::new("", "custom", "::echo ok", Kind::CommandCwd).unwrap())
            .unwrap(),
        Some(Decision::Ask)
    );
}

#[test]
fn web_fetch_requires_canonical_exact_domain_and_exact_category() {
    let policy = rules(&[
        ("*", "*", Decision::Allow),
        ("web_*", "domain:example.com", Decision::Allow),
        ("web_fetch", "*", Decision::Allow),
        ("web_fetch", "domain:*.example.com", Decision::Allow),
        ("web_fetch", "domain:EXAMPLE.COM", Decision::Allow),
        ("web_fetch", "domain:example.com.", Decision::Allow),
        ("web_fetch", "https://example.com", Decision::Allow),
        ("web_fetch", "domain:example.com", Decision::Deny),
        ("web_fetch", "domain:example.com", Decision::Ask),
        ("web_fetch", "domain:[2001:db8::1]", Decision::Allow),
    ]);
    assert_eq!(policy.web_fetch_warning_count(), 5);
    assert_eq!(
        decide(&policy, "web_fetch", "domain:example.com", Kind::Url),
        Some(Decision::Ask)
    );
    assert_eq!(
        decide(&policy, "web_fetch", "domain:[2001:db8::1]", Kind::Url),
        Some(Decision::Allow)
    );
    for target in [
        "example.com",
        "domain:other.com",
        "domain:sub.example.com",
        "domain:example.com.",
        "domain:EXAMPLE.COM",
        "domain:*.example.com",
        "domain:example.com:443",
        "https://example.com",
        "domain:[2001:DB8::1]",
        "domain:[::1%lo0]",
        "domain:[not-ipv6]",
        "domain:例.example",
        "domain:bad..example",
    ] {
        assert_eq!(
            decide(&policy, "web_fetch", target, Kind::Url),
            None,
            "{target}"
        );
    }
    // The pin permits non-DNS-strict labels; do not silently substitute URL/DNS normalization.
    for target in [
        "domain:localhost",
        "domain:-label",
        "domain:127.0.0.1",
        "domain:[::ffff:192.0.2.1]",
    ] {
        assert_eq!(
            decide(
                &rules(&[("web_fetch", target, Decision::Allow)]),
                "web_fetch",
                target,
                Kind::Url
            ),
            Some(Decision::Allow)
        );
    }
}

#[test]
fn config_values_are_trimmed_only_as_pinned_and_size_accounts_json_escaping() {
    let rule = Rule::new(" \tread\r\n", " \ta\r\n", Decision::Allow).unwrap();
    assert_eq!(rule.permission(), "read");
    assert_eq!(rule.pattern(), "a");
    assert_eq!(rule.decision(), Decision::Allow);
    assert_eq!(
        Rule::new(" \t\r\n", "a", Decision::Allow),
        Err(Error::InvalidPermission)
    );
    assert_eq!(
        Rule::new("read", &"\0".repeat(MAX_CONFIG_BYTES / 6), Decision::Allow),
        Err(Error::Limit)
    );
    let empty = Rule::new("a", "", Decision::Ask).unwrap();
    let overhead = serde_json::to_vec(&vec![empty]).unwrap().len();
    let exact = Rule::new("a", &"x".repeat(MAX_CONFIG_BYTES - overhead), Decision::Ask).unwrap();
    let policy = Rules::new(vec![exact.clone()]).unwrap();
    assert_eq!(serde_json::to_vec(&policy).unwrap().len(), MAX_CONFIG_BYTES);
    let too_large = Rule::new(
        "a",
        &"x".repeat(MAX_CONFIG_BYTES - overhead + 1),
        Decision::Ask,
    )
    .unwrap();
    assert_eq!(Rules::new(vec![too_large]), Err(Error::Limit));
    assert_eq!(
        Rules::new(vec![exact, Rule::new("a", "", Decision::Ask).unwrap()]),
        Err(Error::Limit)
    );
    let many = Rules::new(vec![Rule::new("a", "", Decision::Ask).unwrap(); 1025]).unwrap();
    assert_eq!(many.rules().len(), 1025);
}

#[test]
fn target_bounds_and_redaction_do_not_leak_sensitive_inputs() {
    assert_eq!(
        Target::new("/workspace", "bad tool name", "secret", Kind::None).unwrap_err(),
        Error::InvalidTarget
    );
    assert_eq!(
        Target::new("/workspace", "tool", "secret\0", Kind::None).unwrap_err(),
        Error::InvalidTarget
    );
    assert_eq!(
        Target::new(
            &"x".repeat(MAX_TERMINAL_CWD_BYTES + 1),
            "tool",
            "secret",
            Kind::None
        )
        .unwrap_err(),
        Error::Limit
    );
    assert_eq!(
        Target::new(
            "/workspace",
            "tool",
            &"x".repeat(MAX_CONFIGURED_PERMISSION_TARGET_BYTES + 1),
            Kind::None
        )
        .unwrap_err(),
        Error::Limit
    );
    let command = "x".repeat(MAX_CONFIGURED_PERMISSION_TARGET_BYTES);
    let target = Target::new("/workspace", "tool", &command, Kind::None).unwrap();
    let rule = Rule::new("private_category", "secret_token", Decision::Allow).unwrap();
    let policy = Rules::new(vec![rule.clone()]).unwrap();
    for value in [
        format!("{target:?}"),
        format!("{rule:?}"),
        format!("{policy:?}"),
        Error::InvalidTarget.to_string(),
    ] {
        assert!(!value.contains("secret_token"));
        assert!(!value.contains("private_category"));
        assert!(!value.contains("/workspace"));
        assert!(!value.contains(&command));
    }
}

#[test]
fn target_validation_and_ordinary_matching_allocate_nothing() {
    let oversized_name = "a".repeat(1024 * 1024);
    let oversized_target = "b".repeat(MAX_CONFIGURED_PERMISSION_TARGET_BYTES + 1);
    let policy = rules(&[("read", "src/**", Decision::Allow)]);
    let allocations = allocation_counter::measure(|| {
        assert!(Target::new("/workspace", &oversized_name, "a", Kind::None).is_err());
        assert!(Target::new("/workspace", "read_file", &oversized_target, Kind::None).is_err());
        assert_eq!(
            decide(
                &policy,
                "read_file",
                "/workspace/src/file",
                Kind::PathExisting
            ),
            Some(Decision::Allow)
        );
    });
    assert_eq!(allocations.count_total, 0);
}

#[test]
fn pathological_backtracking_has_an_explicit_whole_evaluation_limit() {
    let long = format!("*{}b", "a".repeat(4096));
    let policy = rules(&[
        ("read", "*", Decision::Allow),
        ("read", &long, Decision::Deny),
    ]);
    let target = "a".repeat(8192);
    assert_eq!(
        policy.decide(&Target::new("", "read_file", &target, Kind::None).unwrap()),
        Err(Error::Limit)
    );
    let stars = format!("{}b", "*a".repeat(4096));
    assert_eq!(
        decide(
            &rules(&[("read", &stars, Decision::Allow)]),
            "read_file",
            &target,
            Kind::None
        ),
        None
    );
}

#[test]
fn short_patterns_match_the_pinned_recursive_definition_exhaustively() {
    fn words(alphabet: &[u8], maximum: usize) -> Vec<String> {
        let mut values = vec![String::new()];
        let mut start = 0;
        for _ in 0..maximum {
            let end = values.len();
            for i in start..end {
                for byte in alphabet {
                    let mut word = values[i].clone();
                    word.push(char::from(*byte));
                    values.push(word);
                }
            }
            start = end;
        }
        values
    }
    // The reference is deliberately recursive only for these <=4-byte fixtures.
    fn reference(pattern: &[u8], candidate: &[u8]) -> bool {
        match pattern.first() {
            None => candidate.is_empty(),
            Some(b'*') => {
                reference(&pattern[1..], candidate)
                    || (!candidate.is_empty() && reference(pattern, &candidate[1..]))
            }
            Some(b'?') => !candidate.is_empty() && reference(&pattern[1..], &candidate[1..]),
            Some(byte) => {
                candidate.first() == Some(byte) && reference(&pattern[1..], &candidate[1..])
            }
        }
    }
    let candidates = words(b"ab*", 4);
    for pattern in words(b"ab*?", 4) {
        let policy = rules(&[("read", &pattern, Decision::Allow)]);
        for candidate in &candidates {
            assert_eq!(
                decide(&policy, "read_file", candidate, Kind::None).is_some(),
                reference(pattern.as_bytes(), candidate.as_bytes()),
                "{pattern:?} / {candidate:?}"
            );
        }
    }
}
