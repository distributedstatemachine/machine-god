use std::collections::BTreeMap;

use machine_god_native::{
    AI_GATEWAY_DEFAULT_MODEL, MAX_NATIVE_REASONING_EFFORT_BYTES,
    MAX_NATIVE_REASONING_EFFORT_OPTIONS, NATIVE_MODEL_PREFERENCES_KEY, NativeFastModeChange,
    NativeModelCapabilities, NativeModelPreferences, NativeModelPreferencesError,
    NativeReasoningEffort,
};
use serde_json::{Value, json};

fn effort(name: &str) -> NativeReasoningEffort {
    NativeReasoningEffort::parse(name).unwrap()
}

#[test]
fn effort_aliases_and_opaque_names_preserve_pinned_semantics() {
    for alias in ["auto", "AUTO", "Adaptive", "DEFAULT"] {
        let parsed = effort(alias);
        assert_eq!(parsed, NativeReasoningEffort::default());
        assert_eq!(parsed.as_named(), None);
        assert_eq!(parsed.label(), "auto");
    }
    for name in ["future-tier", "HIGH", "tier_2.7", ".", "-"] {
        assert_eq!(effort(name).as_named(), Some(name));
    }
    assert_ne!(effort("high"), effort("HIGH"));
    assert_eq!(
        effort(&"x".repeat(64)).label().len(),
        MAX_NATIVE_REASONING_EFFORT_BYTES
    );
    for invalid in [
        String::new(),
        "x".repeat(65),
        " high".to_owned(),
        "high ".to_owned(),
        "é".to_owned(),
        "x/y".to_owned(),
        "a\n".to_owned(),
    ] {
        assert_eq!(
            NativeReasoningEffort::parse(&invalid),
            Err(NativeModelPreferencesError::InvalidEffort)
        );
    }
}

#[test]
fn preferences_validate_full_utf8_model_contract_without_normalization() {
    for model in [
        "é".repeat(512),
        "\u{85}provider modèle\u{a0}".to_owned(),
        "m".repeat(1024),
    ] {
        let prefs = NativeModelPreferences::new(&model, effort("high"), true).unwrap();
        assert_eq!(prefs.model().as_bytes(), model.as_bytes());
        assert_eq!(
            NativeModelPreferences::from_value(&prefs.to_value()).unwrap(),
            prefs
        );
    }
    let mut prefs = NativeModelPreferences::default();
    let before = prefs.clone();
    for model in [
        "é".repeat(512) + "x",
        String::new(),
        " model".to_owned(),
        "model ".to_owned(),
        "x\0y".to_owned(),
        "x\u{7f}y".to_owned(),
    ] {
        assert_eq!(
            prefs.set_model(&model),
            Err(NativeModelPreferencesError::InvalidModel)
        );
        assert_eq!(prefs, before);
    }
}

#[test]
fn absence_is_not_explicit_defaults_and_unrelated_metadata_is_preserved() {
    let mut metadata = BTreeMap::from([("unrelated".to_owned(), json!({"private": true}))]);
    assert_eq!(
        NativeModelPreferences::from_metadata(&metadata).unwrap(),
        None
    );
    let defaults = NativeModelPreferences::default();
    assert_eq!(defaults.model(), AI_GATEWAY_DEFAULT_MODEL);
    assert_eq!(defaults.effort().as_named(), None);
    assert!(!defaults.requested_fast());
    metadata.insert(NATIVE_MODEL_PREFERENCES_KEY.to_owned(), defaults.to_value());
    assert_eq!(
        NativeModelPreferences::from_metadata(&metadata).unwrap(),
        Some(defaults)
    );
    assert_eq!(metadata["unrelated"], json!({"private": true}));
}

#[test]
fn capabilities_are_bounded_named_only_preserve_duplicates_and_never_infer_from_ids() {
    let choices = vec![effort("future-tier"); 16];
    let caps = NativeModelCapabilities::new(&choices, true).unwrap();
    assert_eq!(caps.reasoning_efforts(), choices);
    assert_eq!(
        caps.reasoning_efforts().len(),
        MAX_NATIVE_REASONING_EFFORT_OPTIONS
    );
    assert!(caps.supports_fast());
    assert_eq!(
        NativeModelCapabilities::new(&vec![effort("high"); 17], true),
        Err(NativeModelPreferencesError::InvalidCapabilities)
    );
    assert_eq!(
        NativeModelCapabilities::new(&[NativeReasoningEffort::default()], false),
        Err(NativeModelPreferencesError::InvalidCapabilities)
    );
    for model in ["openai/gpt-5", "anthropic/claude-opus", "provider-fast"] {
        let prefs = NativeModelPreferences::new(model, effort("high"), true).unwrap();
        let effective = prefs.effective(&NativeModelCapabilities::default());
        assert!(effective.effort().is_none());
        assert!(!effective.fast());
    }
}

#[test]
fn effective_controls_require_exact_advertisement_without_rewriting_requests() {
    let prefs = NativeModelPreferences::new("provider/model", effort("HIGH"), true).unwrap();
    let lower = NativeModelCapabilities::new(&[effort("high")], false).unwrap();
    assert!(prefs.effective(&lower).effort().is_none());
    assert!(!prefs.effective(&lower).fast());
    let caps = NativeModelCapabilities::new(&[effort("HIGH")], true).unwrap();
    let resolved = prefs.effective(&caps);
    assert_eq!(resolved.model(), prefs.model());
    assert_eq!(resolved.effort(), Some(prefs.effort()));
    assert!(resolved.fast());
    assert_eq!(prefs.effort().label(), "HIGH");
    assert!(prefs.requested_fast());
}

#[test]
fn direct_selection_preserves_controls_and_stale_fast_can_always_be_disabled() {
    let mut prefs = NativeModelPreferences::new("old", effort("high"), true).unwrap();
    prefs.set_model("new").unwrap();
    assert_eq!(prefs.effort(), &effort("high"));
    assert!(prefs.requested_fast());
    let unsupported = NativeModelCapabilities::default();
    assert_eq!(
        prefs.toggle_fast(&unsupported),
        NativeFastModeChange::Disabled
    );
    let snapshot = prefs.clone();
    assert_eq!(
        prefs.toggle_fast(&unsupported),
        NativeFastModeChange::Unsupported
    );
    assert_eq!(prefs, snapshot);
    let supported = NativeModelCapabilities::new(&[], true).unwrap();
    assert_eq!(prefs.toggle_fast(&supported), NativeFastModeChange::Enabled);
    assert!(prefs.requested_fast());
}

#[test]
fn picker_defaults_reset_only_supported_controls_and_invalid_models_are_atomic() {
    let mut prefs = NativeModelPreferences::new("old", effort("high"), false).unwrap();
    let caps = NativeModelCapabilities::new(&[effort("high")], true).unwrap();
    let before = prefs.clone();
    assert!(prefs.select_from_picker(" invalid", &caps).is_err());
    assert_eq!(prefs, before);
    prefs.select_from_picker("new", &caps).unwrap();
    assert_eq!(prefs.effort(), &NativeReasoningEffort::default());
    assert!(prefs.requested_fast());
    prefs.set_effort(effort("future-tier"));
    prefs
        .select_from_picker("unsupported", &NativeModelCapabilities::default())
        .unwrap();
    assert_eq!(prefs.effort(), &effort("future-tier"));
    assert!(prefs.requested_fast());
}

#[test]
fn metadata_requires_exact_fields_types_version_and_bounded_values() {
    for value in [
        Value::Null,
        json!([]),
        json!("private"),
        json!({}),
        json!({"schema_version": 1.0}),
        json!({"schema_version": true}),
        json!({"schema_version":1,"model":"m","effort":"auto","fast_mode":false,"extra":true}),
        json!({"schema_version":1,"model":"m","effort":null,"fast_mode":false}),
        json!({"schema_version":1,"model":"m","effort":"auto","fast_mode":0}),
    ] {
        assert_eq!(
            NativeModelPreferences::from_value(&value),
            Err(NativeModelPreferencesError::Malformed)
        );
    }
    assert_eq!(
        NativeModelPreferences::from_value(&json!({"schema_version": 2})),
        Err(NativeModelPreferencesError::UnsupportedVersion)
    );
    let normalized = NativeModelPreferences::from_value(
        &json!({"schema_version":1,"model":"m","effort":"ADAPTIVE","fast_mode":false}),
    )
    .unwrap();
    assert_eq!(normalized.to_value()["effort"], "auto");
}

#[test]
fn deeply_nested_wrong_scalar_and_unrelated_values_are_never_cloned_or_traversed() {
    let mut deep = Value::String("PRIVATE_DEEP_CONTENT".to_owned());
    for _ in 0..20_000 {
        deep = Value::Array(vec![deep]);
    }
    let mut metadata = BTreeMap::new();
    metadata.insert("unrelated".to_owned(), deep);
    assert_eq!(
        NativeModelPreferences::from_metadata(&metadata).unwrap(),
        None
    );
    let deep = metadata.remove("unrelated").unwrap();
    let mut entry = NativeModelPreferences::default().to_value();
    entry["effort"] = deep;
    metadata.insert(NATIVE_MODEL_PREFERENCES_KEY.to_owned(), entry);
    let error = NativeModelPreferences::from_metadata(&metadata).unwrap_err();
    assert_eq!(error, NativeModelPreferencesError::Malformed);
    assert!(!format!("{error:?} {error}").contains("PRIVATE"));
    // The caller owns hostile input teardown. Drain it iteratively too.
    let mut entry = metadata.remove(NATIVE_MODEL_PREFERENCES_KEY).unwrap();
    let mut value = entry.as_object_mut().unwrap().remove("effort").unwrap();
    while let Value::Array(mut children) = value {
        value = children.pop().unwrap();
    }
}

#[test]
fn preference_capability_and_effective_debug_are_redacted() {
    let secret_effort = effort("PRIVATE_EFFORT");
    let prefs = NativeModelPreferences::new("PRIVATE_MODEL", secret_effort.clone(), true).unwrap();
    let caps = NativeModelCapabilities::new(std::slice::from_ref(&secret_effort), true).unwrap();
    let diagnostic = format!(
        "{prefs:?} {secret_effort:?} {caps:?} {:?}",
        prefs.effective(&caps)
    );
    assert!(!diagnostic.contains("PRIVATE"));
}
