use super::*;
use crate::config::parse_config_bytes;
use serde_json::{Value, json};

fn document() -> Value {
    serde_json::from_slice(&NativeConfig::default().serialize_current().unwrap()).unwrap()
}

fn record(source: &[u8], identity: &[u8], canonical: bool) -> NativeSavedWorkspaceDirectory {
    NativeSavedWorkspaceDirectory::new(source, identity, canonical).unwrap()
}

fn entry() -> Value {
    json!({"workspace_hex":"2f776f726b", "additional_directories":[{
        "source_hex":"2f7361766564", "identity_hex":"2f7265616c", "identity_canonical":false
    }]})
}

fn parse(value: &Value) -> Result<NativeConfig, NativeConfigError> {
    parse_config_bytes(&serde_json::to_vec(value).unwrap())
}

#[test]
fn raw_sources_and_unavailable_identities_roundtrip_without_path_probes() {
    let directory = record(b"/never-created-\xff", b"/also-missing-\xfe", false);
    let (config, changed) = NativeConfig::default()
        .with_workspace_directory_mutation(
            b"/primary-\x80",
            &NativeWorkspaceDirectoryMutation::Add(directory.clone()),
        )
        .unwrap();
    assert!(changed);
    let encoded = config.serialize_current().unwrap();
    let loaded = parse_config_bytes(&encoded).unwrap();
    assert_eq!(loaded, config);
    assert_eq!(
        loaded
            .saved_workspace_directories(b"/primary-\x80")
            .unwrap(),
        std::slice::from_ref(&directory)
    );
    assert_eq!(directory.source_bytes(), b"/never-created-\xff");
    assert_eq!(directory.identity_bytes(), b"/also-missing-\xfe");
    assert!(!directory.identity_canonical());
    let debug = format!("{directory:?} {config:?}");
    assert!(!debug.contains("never-created"));
    assert!(!debug.contains("primary"));
}

#[test]
fn path_values_enforce_normalized_absolute_byte_bounds() {
    for invalid_path in [
        b"".as_slice(),
        b"relative",
        b"//root",
        b"/trailing/",
        b"/./x",
        b"/x/../y",
        b"/nul\0x",
    ] {
        assert!(NativeSavedWorkspaceDirectory::new(invalid_path, b"/valid", true).is_err());
        assert!(NativeSavedWorkspaceDirectory::new(b"/valid", invalid_path, false).is_err());
        assert!(
            NativeConfig::default()
                .saved_workspace_directories(invalid_path)
                .is_err()
        );
    }
    let mut maximal = vec![b'x'; MAX_PATH_BYTES];
    maximal[0] = b'/';
    assert!(NativeSavedWorkspaceDirectory::new(&maximal, &maximal, true).is_ok());
    maximal.push(b'x');
    assert_eq!(
        NativeSavedWorkspaceDirectory::new(&maximal, b"/valid", true)
            .unwrap_err()
            .kind(),
        NativeConfigErrorKind::TooLarge
    );
}

#[test]
fn strict_v7_fields_types_and_duplicates_are_rejected() {
    let mut value = document();
    value["workspace_directories"] = json!([entry()]);
    assert!(parse(&value).is_ok());
    for location in [
        vec![],
        vec!["workspace_directories", "0"],
        vec!["workspace_directories", "0", "additional_directories", "0"],
    ] {
        let mut invalid_value = value.clone();
        let target = match location.len() {
            0 => &mut invalid_value,
            2 => &mut invalid_value["workspace_directories"][0],
            _ => &mut invalid_value["workspace_directories"][0]["additional_directories"][0],
        };
        target["unexpected"] = json!(true);
        assert!(parse(&invalid_value).is_err());
    }
    for key in ["source_hex", "identity_hex", "identity_canonical"] {
        let mut missing = value.clone();
        missing["workspace_directories"][0]["additional_directories"][0]
            .as_object_mut()
            .unwrap()
            .remove(key);
        assert!(parse(&missing).is_err());
        let mut wrong = value.clone();
        wrong["workspace_directories"][0]["additional_directories"][0][key] = Value::Null;
        assert!(parse(&wrong).is_err());
    }
    for (old, new) in [
        (
            "\"workspace_directories\":[",
            "\"workspace_directories\":[],\"workspace_directories\":[",
        ),
        (
            "\"workspace_hex\":",
            "\"workspace_hex\":\"2f78\",\"workspace_hex\":",
        ),
        ("\"source_hex\":", "\"source_hex\":\"2f78\",\"source_hex\":"),
        (
            "\"identity_hex\":",
            "\"identity_hex\":\"2f78\",\"identity_hex\":",
        ),
        (
            "\"identity_canonical\":",
            "\"identity_canonical\":true,\"identity_canonical\":",
        ),
    ] {
        let encoded = serde_json::to_string(&value).unwrap().replacen(old, new, 1);
        assert!(parse_config_bytes(encoded.as_bytes()).is_err());
    }
}

#[test]
fn hex_and_duplicate_primary_source_identity_contracts_are_strict() {
    for encoded in ["", "2F78", "2f7", "zz", "7800", "2f00", "2f2e", "2f2f78"] {
        let mut value = document();
        value["workspace_directories"] = json!([entry()]);
        value["workspace_directories"][0]["additional_directories"][0]["source_hex"] =
            json!(encoded);
        assert!(parse(&value).is_err(), "{encoded}");
    }
    let mut value = document();
    value["workspace_directories"] = json!([entry(), entry()]);
    assert!(parse(&value).is_err());
    value["workspace_directories"] = json!([entry()]);
    let item = value["workspace_directories"][0]["additional_directories"][0].clone();
    for duplicate_identity in [false, true] {
        let mut other = item.clone();
        other[if duplicate_identity {
            "source_hex"
        } else {
            "identity_hex"
        }] = json!("2f6f74686572");
        value["workspace_directories"][0]["additional_directories"] = json!([item, other]);
        assert!(parse(&value).is_err());
    }
}

#[test]
fn primary_rejected_and_sixteen_directory_limit_preserves_order() {
    let primary = b"/work";
    let mut config = NativeConfig::default();
    for directory in [
        record(primary, b"/other", true),
        record(b"/other", primary, false),
    ] {
        assert!(
            config
                .with_workspace_directory_mutation(
                    primary,
                    &NativeWorkspaceDirectoryMutation::Add(directory)
                )
                .is_err()
        );
    }
    for index in 0..MAX_DIRECTORIES {
        let path = format!("/dir-{index}");
        (config, _) = config
            .with_workspace_directory_mutation(
                primary,
                &NativeWorkspaceDirectoryMutation::Add(record(
                    path.as_bytes(),
                    path.as_bytes(),
                    true,
                )),
            )
            .unwrap();
    }
    assert_eq!(
        config.saved_workspace_directories(primary).unwrap()[15].identity_bytes(),
        b"/dir-15"
    );
    assert!(
        config
            .with_workspace_directory_mutation(
                primary,
                &NativeWorkspaceDirectoryMutation::Add(record(b"/overflow", b"/overflow", true))
            )
            .is_err()
    );
    let duplicate = NativeWorkspaceDirectoryMutation::Add(record(b"/alias", b"/dir-0", true));
    let (same, changed) = config
        .with_workspace_directory_mutation(primary, &duplicate)
        .unwrap();
    assert!(!changed);
    assert_eq!(same, config);
}

#[test]
fn exact_identity_removal_never_retargets_source_and_clear_removes_only_selection() {
    let primary = b"/work";
    let first = record(b"/symlink", b"/original", true);
    let (mut config, _) = NativeConfig::default()
        .with_workspace_directory_mutation(
            primary,
            &NativeWorkspaceDirectoryMutation::Add(first.clone()),
        )
        .unwrap();
    (config, _) = config
        .with_workspace_directory_mutation(
            b"/other",
            &NativeWorkspaceDirectoryMutation::Add(record(b"/elsewhere", b"/elsewhere", false)),
        )
        .unwrap();
    assert!(
        config
            .with_workspace_directory_mutation(
                primary,
                &NativeWorkspaceDirectoryMutation::Add(record(b"/symlink", b"/retargeted", true))
            )
            .is_err()
    );
    let (same, changed) = config
        .with_workspace_directory_mutation(
            primary,
            &NativeWorkspaceDirectoryMutation::Remove(b"/symlink".to_vec()),
        )
        .unwrap();
    assert!(!changed);
    assert_eq!(same, config);
    let (removed, changed) = config
        .with_workspace_directory_mutation(
            primary,
            &NativeWorkspaceDirectoryMutation::Remove(b"/original".to_vec()),
        )
        .unwrap();
    assert!(changed);
    assert!(
        removed
            .saved_workspace_directories(primary)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        removed
            .saved_workspace_directories(b"/other")
            .unwrap()
            .len(),
        1
    );
    let (cleared, _) = config
        .with_workspace_directory_mutation(primary, &NativeWorkspaceDirectoryMutation::Clear)
        .unwrap();
    assert_eq!(cleared, removed);
}

#[test]
fn complete_envelope_bound_applies_across_workspaces() {
    let mut source = vec![b's'; 4000];
    source[0] = b'/';
    let mut identity = vec![b'i'; 4000];
    identity[0] = b'/';
    let record = record(&source, &identity, false);
    let mut config = NativeConfig::default();
    for index in 0..4 {
        (config, _) = config
            .with_workspace_directory_mutation(
                format!("/work-{index}").as_bytes(),
                &NativeWorkspaceDirectoryMutation::Add(record.clone()),
            )
            .unwrap();
    }
    assert_eq!(
        config
            .with_workspace_directory_mutation(
                b"/one-more",
                &NativeWorkspaceDirectoryMutation::Add(record)
            )
            .unwrap_err()
            .kind(),
        NativeConfigErrorKind::TooLarge
    );
}

#[test]
fn v6_remains_strict_and_upgrades_only_on_changed_edit() {
    let mut value = document();
    value["schema_version"] = json!(6);
    assert!(parse(&value).is_err());
    value
        .as_object_mut()
        .unwrap()
        .remove("workspace_directories");
    let legacy = parse(&value).unwrap();
    assert_eq!(legacy.schema_version(), 6);
    let (same, changed) = legacy
        .with_workspace_directory_mutation(b"/work", &NativeWorkspaceDirectoryMutation::Clear)
        .unwrap();
    assert!(!changed);
    assert_eq!(same, legacy);
    let (changed, _) = legacy
        .with_workspace_directory_mutation(
            b"/work",
            &NativeWorkspaceDirectoryMutation::Add(record(b"/new", b"/new", true)),
        )
        .unwrap();
    assert_eq!(changed.schema_version(), 7);
}

#[test]
fn wire_capacity_primary_exclusion_and_required_collection_are_enforced() {
    let mut value = document();
    value
        .as_object_mut()
        .unwrap()
        .remove("workspace_directories");
    assert!(parse(&value).is_err());
    for count in [16, 17] {
        value["workspace_directories"] = json!([{
            "workspace_hex": "2f776f726b",
            "additional_directories": (0..count).map(|index| {
                let path = format!("/extra-{index}");
                json!({"source_hex":encode_hex(path.as_bytes()), "identity_hex":encode_hex(path.as_bytes()), "identity_canonical":false})
            }).collect::<Vec<_>>()
        }]);
        assert_eq!(parse(&value).is_ok(), count == 16);
    }
    for key in ["source_hex", "identity_hex"] {
        value["workspace_directories"] = json!([entry()]);
        value["workspace_directories"][0]["additional_directories"][0][key] = json!("2f776f726b");
        assert!(parse(&value).is_err());
    }
    value["workspace_directories"] =
        json!([{"workspace_hex":"2f776f726b", "additional_directories":[]}]);
    let config = parse(&value).unwrap();
    let (same, changed) = config
        .with_workspace_directory_mutation(b"/work", &NativeWorkspaceDirectoryMutation::Clear)
        .unwrap();
    assert!(!changed);
    assert_eq!(same, config);
}
