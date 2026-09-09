use super::*;
use crate::config::parse_config_bytes;
use serde_json::{Value, json};
use std::os::unix::ffi::OsStrExt;

fn document() -> Value {
    serde_json::from_slice(&NativeConfig::default().serialize_current().unwrap()).unwrap()
}
fn parsed(value: &Value) -> NativeConfig {
    parse_config_bytes(&serde_json::to_vec(value).unwrap()).unwrap()
}
fn rule(permission: &str, pattern: &str, action: &str) -> Value {
    json!({"permission":permission,"pattern":pattern,"action":action})
}
fn edit(
    config: &NativeConfig,
    mutation: &NativeConfiguredPermissionMutation,
) -> (NativeConfig, NativeConfiguredPermissionMutationOutcome) {
    config
        .with_permission_mutation(
            Path::new("/work"),
            NativeConfiguredPermissionScope::Local,
            mutation,
        )
        .unwrap()
}
fn add(permission: &str, pattern: &str) -> NativeConfiguredPermissionMutation {
    NativeConfiguredPermissionMutation::Add {
        permission: permission.into(),
        pattern: pattern.into(),
    }
}

#[test]
fn absent_empty_and_populated_sources_are_distinct_and_lossless() {
    let mut value = document();
    value["permission_rules"] = json!([rule("bash", "user *", "allow")]);
    let config = parsed(&value);
    assert!(!config.has_workspace_permission_rules());
    let sources = config.permission_sources(Path::new("/work")).unwrap();
    assert!(!sources.user_shadowed_by_local());
    assert_eq!(sources.effective(), sources.user());
    value["workspace_permission_rules"] = json!([
        {"workspace_hex":"2f776f726b","permission_rules":[]},
        {"workspace_hex":"2fff","permission_rules":[rule("read", "*", "allow")]}
    ]);
    let config = parsed(&value);
    assert!(config.has_workspace_permission_rules());
    let sources = config.permission_sources(Path::new("/work")).unwrap();
    assert!(sources.user_shadowed_by_local());
    assert!(sources.effective().rules().is_empty());
    assert_eq!(sources.user().rules()[0].pattern(), "user *");
    let path = Path::new(std::ffi::OsStr::from_bytes(b"/\xff"));
    assert_eq!(
        config.permission_sources(path).unwrap().effective().rules()[0].permission(),
        "read"
    );
    assert_eq!(
        config,
        parse_config_bytes(&config.serialize_current().unwrap()).unwrap()
    );
    assert!(!format!("{sources:?}{config:?}").contains("user *"));
}

#[test]
fn strict_v6_rejects_bad_paths_duplicates_unknowns_and_missing_fields() {
    for key in [
        "",
        "2F",
        "00",
        "61",
        "2f2f61",
        "2f612f",
        "2f2e",
        "2f2e2e",
        "2f610062",
        "2f0",
        "zz",
        &format!("2f{}", "61".repeat(4096)),
    ] {
        let mut value = document();
        value["workspace_permission_rules"] = json!([{"workspace_hex":key,"permission_rules":[]}]);
        assert!(
            parse_config_bytes(&serde_json::to_vec(&value).unwrap()).is_err(),
            "{key}"
        );
    }
    for entry in [
        json!({"workspace_hex":"2f"}),
        json!({"permission_rules":[]}),
        json!({"workspace_hex":"2f","permission_rules":null}),
        json!({"workspace_hex":"2f","permission_rules":[],"extra":1}),
    ] {
        let mut value = document();
        value["workspace_permission_rules"] = json!([entry]);
        assert!(parse_config_bytes(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    let mut value = document();
    value["workspace_permission_rules"] = json!([{"workspace_hex":"2f","permission_rules":[]},{"workspace_hex":"2f","permission_rules":[]}]);
    assert!(parse_config_bytes(&serde_json::to_vec(&value).unwrap()).is_err());
    let encoded = serde_json::to_string(&document()).unwrap().replace("\"workspace_permission_rules\":[]", "\"workspace_permission_rules\":[{\"workspace_hex\":\"2f\",\"workspace_hex\":\"2f\",\"permission_rules\":[]}]");
    assert!(parse_config_bytes(encoded.as_bytes()).is_err());
    let mut value = document();
    value
        .as_object_mut()
        .unwrap()
        .remove("workspace_permission_rules");
    assert!(parse_config_bytes(&serde_json::to_vec(&value).unwrap()).is_err());
    value["schema_version"] = json!(5);
    value
        .as_object_mut()
        .unwrap()
        .remove("workspace_directories");
    assert_eq!(parsed(&value).schema_version(), 5);
    value["workspace_permission_rules"] = json!([]);
    assert!(parse_config_bytes(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn remove_last_keeps_empty_shadow_and_empty_reset_is_unchanged() {
    let mut value = document();
    value["permission_rules"] = json!([rule("bash", "user *", "allow")]);
    let (config, outcome) = edit(&parsed(&value), &add("bash", "local *"));
    assert_eq!(
        outcome,
        NativeConfiguredPermissionMutationOutcome::Changed { removed_rules: 0 }
    );
    let (empty, outcome) = edit(
        &config,
        &NativeConfiguredPermissionMutation::Remove {
            permission: "bash".into(),
            pattern: "local *".into(),
        },
    );
    assert_eq!(
        outcome,
        NativeConfiguredPermissionMutationOutcome::Changed { removed_rules: 1 }
    );
    assert!(
        empty
            .permission_sources(Path::new("/work"))
            .unwrap()
            .user_shadowed_by_local()
    );
    let (same, outcome) = edit(
        &empty,
        &NativeConfiguredPermissionMutation::Reset(NativeConfiguredPermissionReset::All),
    );
    assert_eq!(
        outcome,
        NativeConfiguredPermissionMutationOutcome::Unchanged
    );
    assert_eq!(same, empty);
    let (revealed, outcome) = edit(
        &config,
        &NativeConfiguredPermissionMutation::Reset(NativeConfiguredPermissionReset::All),
    );
    assert_eq!(
        outcome,
        NativeConfiguredPermissionMutationOutcome::Changed { removed_rules: 1 }
    );
    let sources = revealed.permission_sources(Path::new("/work")).unwrap();
    assert!(sources.local().is_none());
    assert_eq!(sources.effective().rules()[0].pattern(), "user *");
}

#[test]
fn add_updates_last_duplicate_remove_removes_decisions_and_reset_preserves_policy() {
    let mut value = document();
    value["workspace_permission_rules"] = json!([{"workspace_hex":"2f776f726b","permission_rules":[
        rule("read", "*", "deny"), rule("read", "*", "ask"), rule("edit", "*", "deny")
    ]}]);
    let (config, _) = edit(&parsed(&value), &add("read", "*"));
    let rules = config
        .permission_sources(Path::new("/work"))
        .unwrap()
        .effective()
        .rules();
    assert_eq!(
        rules[0].decision(),
        NativeConfiguredPermissionDecision::Deny
    );
    assert_eq!(
        rules[1].decision(),
        NativeConfiguredPermissionDecision::Allow
    );
    assert_eq!(rules[2].permission(), "edit");
    assert_eq!(
        edit(&config, &add("read", "*")).1,
        NativeConfiguredPermissionMutationOutcome::Unchanged
    );
    let (reset, _) = edit(
        &config,
        &NativeConfiguredPermissionMutation::Reset(NativeConfiguredPermissionReset::All),
    );
    assert_eq!(
        reset
            .permission_sources(Path::new("/work"))
            .unwrap()
            .effective()
            .rules()
            .len(),
        2
    );
    let (removed, outcome) = edit(
        &config,
        &NativeConfiguredPermissionMutation::Remove {
            permission: "read".into(),
            pattern: "*".into(),
        },
    );
    assert_eq!(
        outcome,
        NativeConfiguredPermissionMutationOutcome::Changed { removed_rules: 2 }
    );
    assert_eq!(
        removed
            .permission_sources(Path::new("/work"))
            .unwrap()
            .effective()
            .rules()[0]
            .permission(),
        "edit"
    );
}

#[test]
fn every_reset_scope_preserves_excluded_categories_and_nonallow_rules() {
    let categories = [
        "bash",
        "url",
        "open_url",
        "browser_navigate",
        "web_fetch",
        "*",
        "read",
        "custom",
    ];
    for (scope, expected) in [
        (NativeConfiguredPermissionReset::Commands, 1),
        (NativeConfiguredPermissionReset::Urls, 3),
        (NativeConfiguredPermissionReset::WebFetchDomains, 1),
        (NativeConfiguredPermissionReset::Tools, 2),
        (NativeConfiguredPermissionReset::All, 8),
    ] {
        let mut value = document();
        let rules: Vec<_> = categories
            .iter()
            .flat_map(|category| {
                [
                    rule(category, "*", "allow"),
                    rule(category, "deny", "deny"),
                    rule(category, "ask", "ask"),
                ]
            })
            .collect();
        value["workspace_permission_rules"] =
            json!([{"workspace_hex":"2f776f726b","permission_rules":rules}]);
        let (config, outcome) = edit(
            &parsed(&value),
            &NativeConfiguredPermissionMutation::Reset(scope),
        );
        assert_eq!(
            outcome,
            NativeConfiguredPermissionMutationOutcome::Changed {
                removed_rules: expected
            }
        );
        assert_eq!(
            config
                .permission_sources(Path::new("/work"))
                .unwrap()
                .effective()
                .rules()
                .len(),
            24 - expected
        );
    }
}

#[test]
fn aggregate_envelope_and_workspace_bounds_apply_before_cloning_edits() {
    let config = NativeConfig::default();
    for path in ["relative", "/work/", "/work/../other", "//work", "/./work"] {
        assert!(config.permission_sources(Path::new(path)).is_err());
    }
    let boundary = format!("/{}", "x".repeat(4095));
    assert!(config.permission_sources(Path::new(&boundary)).is_ok());
    assert!(
        config
            .permission_sources(Path::new(&format!("{boundary}x")))
            .is_err()
    );
    let mutation = add("bash", &"x".repeat(MAX_CONFIG_BYTES));
    assert!(
        config
            .with_permission_mutation(
                Path::new("/work"),
                NativeConfiguredPermissionScope::Local,
                &mutation
            )
            .is_err()
    );
    for mutation in [add(" bash", "x"), add("bash", "x "), add("", "x")] {
        assert!(
            config
                .with_permission_mutation(
                    Path::new("/work"),
                    NativeConfiguredPermissionScope::User,
                    &mutation
                )
                .is_err()
        );
    }
    let (first, _) = edit(&config, &add("bash", &"a".repeat(33_000)));
    assert!(
        first
            .with_permission_mutation(
                Path::new("/other"),
                NativeConfiguredPermissionScope::Local,
                &add("bash", &"b".repeat(33_000))
            )
            .is_err()
    );
}
