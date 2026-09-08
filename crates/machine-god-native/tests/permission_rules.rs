use std::collections::BTreeMap;

use machine_god_native::{
    MAX_FILE_SESSION_BYTES, NATIVE_SESSION_PERMISSION_RULES_KEY,
    NativePermissionRuleDecision as Decision, NativePermissionRuleError as Error,
    NativePermissionRuleKey as Key, NativePermissionRuleKind as Kind,
    NativeSessionPermissionRules as Rules,
};
use serde_json::{Value, json};

fn key(kind: Kind, value: &str) -> Key {
    Key::new(kind, value).unwrap()
}
fn one() -> Rules {
    Rules::default()
        .apply_set(
            &key(Kind::Command, "command\0/workspace\0git status"),
            "git status in /workspace",
            Decision::Allow,
            None,
        )
        .unwrap()
}
fn fixture(count: usize, canonical_size: usize, display: &str) -> Value {
    json!({
        "schema_version": 1,
        "next_generation": count + 1,
        "rules": (0..count).map(|index| {
            let prefix = format!("{index:04}");
            let canonical = prefix + &"x".repeat(canonical_size.saturating_sub(4));
            json!({"id": index + 1, "key": key(Kind::Command, &canonical),
                "display_identity": display, "decision": "allow", "generation": index + 1})
        }).collect::<Vec<_>>()
    })
}

#[test]
fn absence_is_empty_without_reading_unrelated_metadata() {
    let mut deep = Value::Null;
    for _ in 0..4096 {
        deep = Value::Array(vec![deep]);
    }
    let mut metadata = BTreeMap::from([("unrelated".to_owned(), deep)]);
    assert_eq!(Rules::from_metadata(&metadata).unwrap(), Rules::default());
    metadata.insert(
        NATIVE_SESSION_PERMISSION_RULES_KEY.to_owned(),
        one().to_value(),
    );
    assert_eq!(Rules::from_metadata(&metadata).unwrap(), one());
    metadata.insert(NATIVE_SESSION_PERMISSION_RULES_KEY.to_owned(), Value::Null);
    assert_eq!(Rules::from_metadata(&metadata), Err(Error::Malformed));
    drop_iteratively(metadata.remove("unrelated").unwrap());
}

fn drop_iteratively(mut value: Value) {
    loop {
        match value {
            Value::Array(mut values) if values.len() == 1 => value = values.pop().unwrap(),
            _ => break,
        }
    }
}

#[test]
fn canonical_identity_is_exact_and_display_never_matches() {
    let first = key(Kind::Command, "command\0/workspace\0git status");
    let rules = one();
    assert_eq!(rules.decide(&first), Some(Decision::Allow));
    assert_eq!(rules.rule_for_id(1), rules.rule_for_key(&first));
    assert_eq!(
        rules.rule_for_key(&first).unwrap().display_identity(),
        "git status in /workspace"
    );
    for candidate in [
        key(Kind::Command, "command\0/workspace\0git status --short"),
        key(Kind::Command, "git status in /workspace"),
        key(Kind::FileMutation, first.canonical()),
        key(Kind::StructuredTool, first.canonical()),
    ] {
        assert_eq!(rules.decide(&candidate), None);
    }
    assert_eq!(
        first.digest(),
        key(Kind::FileMutation, first.canonical()).digest()
    );
    assert_eq!(first.kind(), Kind::Command);
}

#[test]
fn all_kinds_and_decisions_round_trip_with_controls_and_unicode() {
    let mut rules = Rules::default();
    for kind in [Kind::Command, Kind::FileMutation, Kind::StructuredTool] {
        for decision in [Decision::Allow, Decision::Deny] {
            let canonical = format!("{decision:?}\0résumé\n\u{1b}");
            rules = rules
                .apply_set(&key(kind, &canonical), "display\0\r\u{1b}é", decision, None)
                .unwrap();
        }
    }
    let value = rules.to_value();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(Rules::from_value(&value).unwrap(), rules);
    let encoded = serde_json::to_string(&value).unwrap();
    assert_eq!(
        Rules::from_value(&serde_json::from_str(&encoded).unwrap()).unwrap(),
        rules
    );
    assert_eq!(rules.rules().len(), 6);
    assert_eq!(rules.next_generation(), 7);
}

#[test]
fn replacements_preserve_ids_and_ignore_unrelated_rule_generations() {
    let initial = one();
    let original = &initial.rules()[0];
    let added = initial
        .apply_set(
            &key(Kind::Command, "other"),
            original.display_identity(),
            Decision::Deny,
            None,
        )
        .unwrap();
    assert_eq!(added.next_generation(), 3);
    let updated = added
        .apply_set(
            original.key(),
            "new display",
            Decision::Deny,
            Some(original.generation()),
        )
        .unwrap();
    assert_eq!(updated.rules()[0].id(), original.id());
    assert_eq!(updated.rules()[0].generation(), 3);
    assert_eq!(updated.next_generation(), 4);
    assert_eq!(updated.rules()[1], added.rules()[1]);
    assert_eq!(initial.rules()[0].decision(), Decision::Allow);
    assert_eq!(added.rules()[0], *original);
    assert_eq!(updated.rules()[0].display_identity(), "new display");
    assert_eq!(
        updated.apply_set(original.key(), "stale", Decision::Allow, Some(1)),
        Err(Error::Stale)
    );
}

#[test]
fn stale_set_and_revoke_are_atomic_and_id_removal_is_ordered() {
    let rules = Rules::from_value(&fixture(3, 8, "same display")).unwrap();
    let second = &rules.rules()[1];
    for expected in [None, Some(0), Some(3), Some(4)] {
        assert_eq!(
            rules.apply_set(second.key(), "replacement", Decision::Deny, expected),
            Err(Error::Stale)
        );
    }
    assert_eq!(
        rules.apply_set(
            &key(Kind::Command, "absent"),
            "display",
            Decision::Deny,
            Some(1)
        ),
        Err(Error::Stale)
    );
    for (id, generation) in [(0, 0), (999, 1), (2, 1), (2, 3), (2, 0)] {
        assert_eq!(rules.apply_revoke(id, generation), Err(Error::Stale));
    }
    let removed = rules
        .apply_revoke(second.id(), second.generation())
        .unwrap();
    assert_eq!(
        removed
            .rules()
            .iter()
            .map(machine_god_native::NativePermissionRule::id)
            .collect::<Vec<_>>(),
        [1, 3]
    );
    assert_eq!(removed.next_generation(), 5);
    assert_eq!(removed.rule_for_id(2), None);
    assert_eq!(rules.rules().len(), 3);
    let inserted = removed
        .apply_set(
            &key(Kind::Command, "changed identity"),
            "same display",
            Decision::Allow,
            None,
        )
        .unwrap();
    assert_eq!(inserted.rules()[2].id(), 5);
    assert_eq!(inserted.rules()[2].generation(), 5);
}

#[test]
fn full_capacity_rejects_insertions_but_allows_replacement_and_removal() {
    let rules = Rules::from_value(&fixture(1024, 8, "display")).unwrap();
    let first = &rules.rules()[0];
    assert_eq!(
        rules.apply_set(&key(Kind::Command, "new"), "display", Decision::Allow, None),
        Err(Error::Full)
    );
    assert_eq!(
        rules.apply_set(
            &key(Kind::Command, "new"),
            "display",
            Decision::Allow,
            Some(2)
        ),
        Err(Error::Stale)
    );
    let updated = rules
        .apply_set(
            first.key(),
            "changed",
            Decision::Deny,
            Some(first.generation()),
        )
        .unwrap();
    assert_eq!(updated.rules().len(), 1024);
    assert_eq!(updated.rules()[0].id(), 1);
    assert_eq!(updated.rules()[0].generation(), 1025);
    assert_eq!(updated.apply_revoke(1, 1025).unwrap().rules().len(), 1023);
    assert_eq!(
        Rules::from_value(&fixture(1025, 8, "display")),
        Err(Error::Full)
    );
}

#[test]
fn generation_exhaustion_preserves_upstream_error_order_and_state() {
    let mut value = one().to_value();
    value["next_generation"] = json!(u64::MAX);
    let rules = Rules::from_value(&value).unwrap();
    for expected in [None, Some(1), Some(2)] {
        assert_eq!(
            rules.apply_set(rules.rules()[0].key(), "display", Decision::Deny, expected),
            Err(Error::GenerationExhausted)
        );
    }
    assert_eq!(rules.apply_revoke(1, 1), Err(Error::GenerationExhausted));
    assert_eq!(rules.apply_revoke(1, 2), Err(Error::Stale));
    assert_eq!(rules.apply_revoke(2, 1), Err(Error::Stale));
    assert_eq!(rules.to_value(), value);
    value["next_generation"] = json!(u64::MAX - 1);
    let rules = Rules::from_value(&value).unwrap();
    let last = rules
        .apply_set(
            &key(Kind::Command, "last"),
            "display",
            Decision::Allow,
            None,
        )
        .unwrap();
    assert_eq!(last.rules()[1].id(), u64::MAX - 1);
    assert_eq!(last.next_generation(), u64::MAX);
    assert_eq!(Rules::from_value(&last.to_value()).unwrap(), last);
}

#[test]
fn identity_limits_are_utf8_bytes_without_extra_normalization() {
    for text in [String::new(), "a".repeat(4097), "é".repeat(2049)] {
        assert_eq!(Key::new(Kind::Command, &text), Err(Error::InvalidIdentity));
        assert_eq!(
            Rules::default().apply_set(&key(Kind::Command, "valid"), &text, Decision::Allow, None),
            Err(Error::InvalidDisplayIdentity)
        );
    }
    for text in [
        "a".repeat(4096),
        "é".repeat(2048),
        "\0\r\n\t\u{1b}".to_owned(),
        " ".to_owned(),
    ] {
        let key = key(Kind::FileMutation, &text);
        let rules = Rules::default()
            .apply_set(&key, &text, Decision::Allow, None)
            .unwrap();
        assert_eq!(Rules::from_value(&rules.to_value()).unwrap(), rules);
        assert_eq!(rules.rules()[0].key().canonical(), text);
    }
}

#[test]
fn schema_is_strict_at_every_shallow_level() {
    let original = one().to_value();
    for invalid in [
        Value::Null,
        json!([]),
        json!(true),
        json!(1),
        json!("secret"),
    ] {
        assert_eq!(Rules::from_value(&invalid), Err(Error::Malformed));
    }
    for pointer in ["", "/rules/0", "/rules/0/key"] {
        let object = original.pointer(pointer).unwrap().as_object().unwrap();
        for field in object.keys() {
            let mut value = original.clone();
            value
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(
                Rules::from_value(&value).is_err(),
                "missing {pointer}/{field}"
            );
        }
        let mut value = original.clone();
        value
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("extra".to_owned(), Value::Null);
        assert_eq!(Rules::from_value(&value), Err(Error::Malformed));
    }
    for pointer in [
        "/schema_version",
        "/next_generation",
        "/rules",
        "/rules/0",
        "/rules/0/id",
        "/rules/0/generation",
        "/rules/0/display_identity",
        "/rules/0/decision",
        "/rules/0/key",
        "/rules/0/key/kind",
        "/rules/0/key/canonical",
        "/rules/0/key/digest",
    ] {
        for invalid in [Value::Null, json!([null]), json!({})] {
            let mut value = original.clone();
            *value.pointer_mut(pointer).unwrap() = invalid;
            assert_eq!(
                Rules::from_value(&value),
                Err(Error::Malformed),
                "{pointer}"
            );
        }
    }
    for version in [0, 2, 256, u64::MAX] {
        let mut value = original.clone();
        value["schema_version"] = json!(version);
        assert_eq!(Rules::from_value(&value), Err(Error::UnsupportedVersion));
    }
}

#[test]
fn invalid_generations_enums_identities_and_digests_are_rejected() {
    let original = one().to_value();
    for (pointer, values) in [
        (
            "/next_generation",
            vec![json!(0), json!(1), json!(-1), json!(2.0)],
        ),
        (
            "/rules/0/id",
            vec![json!(0), json!(2), json!(-1), json!(1.0)],
        ),
        (
            "/rules/0/generation",
            vec![json!(0), json!(2), json!(-1), json!(1.0)],
        ),
        (
            "/rules/0/decision",
            vec![json!("unresolved"), json!("ALLOW")],
        ),
        ("/rules/0/key/kind", vec![json!("Command"), json!("file")]),
        (
            "/rules/0/key/canonical",
            vec![json!(""), json!("x".repeat(4097)), json!("different")],
        ),
        (
            "/rules/0/display_identity",
            vec![json!(""), json!("x".repeat(4097))],
        ),
        (
            "/rules/0/key/digest",
            vec![
                json!("0".repeat(64)),
                json!("g".repeat(64)),
                json!("0".repeat(63)),
                json!(
                    original["rules"][0]["key"]["digest"]
                        .as_str()
                        .unwrap()
                        .to_uppercase()
                ),
            ],
        ),
    ] {
        for invalid in values {
            let mut value = original.clone();
            *value.pointer_mut(pointer).unwrap() = invalid;
            assert!(Rules::from_value(&value).is_err(), "{pointer}");
        }
    }
}

#[test]
fn duplicate_keys_and_ids_reject_but_display_and_generation_may_repeat() {
    let mut value = fixture(2, 8, "same display");
    value["rules"][1]["generation"] = json!(2);
    value["rules"][0]["generation"] = json!(2);
    assert!(Rules::from_value(&value).is_ok());
    let mut duplicate = value.clone();
    duplicate["rules"][1]["id"] = json!(1);
    assert_eq!(Rules::from_value(&duplicate), Err(Error::Malformed));
    value["rules"][1]["key"] = value["rules"][0]["key"].clone();
    assert_eq!(Rules::from_value(&value), Err(Error::Malformed));
    value["rules"][1]["key"]["kind"] = json!("file_mutation");
    assert!(Rules::from_value(&value).is_ok());
}

#[test]
fn deeply_nested_malformed_fields_are_not_walked_or_cloned() {
    for pointer in [
        "/rules",
        "/rules/0/key",
        "/rules/0/key/canonical",
        "/rules/0/display_identity",
    ] {
        let mut deep = Value::Null;
        for _ in 0..4096 {
            deep = Value::Array(vec![deep]);
        }
        let mut value = one().to_value();
        *value.pointer_mut(pointer).unwrap() = deep;
        assert_eq!(Rules::from_value(&value), Err(Error::Malformed));
        let deep = value.pointer_mut(pointer).unwrap().take();
        drop_iteratively(deep);
    }
}

#[test]
fn maximum_rule_and_text_counts_fit_without_arbitrary_stricter_limits() {
    let value = fixture(1024, 4096, &"d".repeat(4096));
    assert!(serde_json::to_vec(&value).unwrap().len() < MAX_FILE_SESSION_BYTES);
    let rules = Rules::from_value(&value).unwrap();
    assert_eq!(rules.rules().len(), 1024);
    assert_eq!(rules.to_value(), value);
}

#[test]
fn rejected_large_values_do_not_clone_identity_payloads() {
    let mut value = fixture(1024, 4096, &"d".repeat(4096));
    value["rules"][1023]["decision"] = json!("invalid");
    let allocations = allocation_counter::measure(|| {
        assert_eq!(Rules::from_value(&value), Err(Error::Malformed));
    });
    // Validation bookkeeping may allocate, but not the eight MiB of identities.
    assert!(
        allocations.bytes_total < u64::try_from(MAX_FILE_SESSION_BYTES / 8).unwrap(),
        "{allocations:?}"
    );
    value["rules"][1023]["decision"] = json!("allow");
    for rule in value["rules"].as_array_mut().unwrap() {
        rule["display_identity"] = json!("\0".repeat(4096));
    }
    let allocations = allocation_counter::measure(|| {
        assert_eq!(Rules::from_value(&value), Err(Error::Limit));
    });
    assert!(
        allocations.bytes_total < u64::try_from(MAX_FILE_SESSION_BYTES / 8).unwrap(),
        "{allocations:?}"
    );
}

#[test]
fn exact_serialized_session_bound_is_enforced_before_candidate_text_clones() {
    let mut value = fixture(1024, 4096, &"d".repeat(4096));
    let initial_bytes = serde_json::to_vec(&value).unwrap().len();
    let mut remaining = MAX_FILE_SESSION_BYTES - initial_bytes;
    for rule in value["rules"].as_array_mut().unwrap() {
        let controls = (remaining / 5).min(4096);
        let slashes = (remaining - controls * 5).min(4096 - controls);
        let added = controls * 5 + slashes;
        // A NUL costs five extra encoded bytes; a backslash costs one.
        rule["display_identity"] = json!(
            "\0".repeat(controls) + &"\\".repeat(slashes) + &"d".repeat(4096 - controls - slashes)
        );
        remaining -= added;
        if remaining == 0 {
            break;
        }
    }
    assert_eq!(remaining, 0);
    assert_eq!(
        serde_json::to_vec(&value).unwrap().len(),
        MAX_FILE_SESSION_BYTES
    );
    let rules = Rules::from_value(&value).unwrap();
    let last = rules.rules().last().unwrap();
    let overflow_display = "\\".to_owned() + &last.display_identity()[1..];
    assert_eq!(
        rules.apply_set(
            last.key(),
            &overflow_display,
            Decision::Allow,
            Some(last.generation())
        ),
        Err(Error::Limit)
    );
    assert_eq!(rules.to_value(), value);
    let final_index = value["rules"].as_array().unwrap().len() - 1;
    value["rules"][final_index]["display_identity"] = json!(overflow_display);
    assert_eq!(Rules::from_value(&value), Err(Error::Limit));
    assert!(rules.apply_revoke(last.id(), last.generation()).is_ok());
}

#[test]
fn debug_and_errors_redact_identity_display_and_digest() {
    let secret = "private-secret-workspace-path";
    let key = key(Kind::Command, secret);
    let rules = Rules::default()
        .apply_set(&key, secret, Decision::Deny, None)
        .unwrap();
    let digest = rules.to_value()["rules"][0]["key"]["digest"]
        .as_str()
        .unwrap()
        .to_owned();
    let debug = format!("{key:?} {rules:?} {:?}", rules.rules()[0]);
    assert!(!debug.contains(secret));
    assert!(!debug.contains(&digest));
    for error in [
        Error::InvalidIdentity,
        Error::InvalidDisplayIdentity,
        Error::Malformed,
        Error::UnsupportedVersion,
        Error::Stale,
        Error::Full,
        Error::GenerationExhausted,
        Error::Limit,
    ] {
        assert!(!format!("{error} {error:?}").contains(secret));
    }
}
