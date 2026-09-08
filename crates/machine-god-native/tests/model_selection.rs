use machine_god_core::{AvailableModel, ModelCatalog, ModelCatalogAccess};
use machine_god_native::{
    MAX_NATIVE_MODEL_QUERY_BYTES, NativeModelSelectionError, resolve_model_query,
};

fn catalog(ids: &[&str]) -> ModelCatalog {
    ModelCatalog::new(
        ids.iter()
            .map(|id| AvailableModel::new(*id).unwrap())
            .collect(),
        ModelCatalogAccess::Authenticated,
    )
}

#[test]
fn exact_ascii_case_insensitive_match_precedes_fuzzy_and_keeps_first_spelling() {
    let models = catalog(&["alpha/foo", "FOO", "foo"]);
    assert_eq!(
        resolve_model_query("foo", &models).unwrap().as_deref(),
        Some("FOO")
    );
}

#[test]
fn fuzzy_scores_preserve_first_highest_and_case_sensitive_prefix_quirk() {
    let models = catalog(&["MODEL/first", "model/second", "provider/MODEL"]);
    assert_eq!(
        resolve_model_query("model", &models).unwrap().as_deref(),
        Some("model/second")
    );
    let models = catalog(&["a/first-name", "b/first-name"]);
    assert_eq!(
        resolve_model_query("first-name", &models)
            .unwrap()
            .as_deref(),
        Some("a/first-name")
    );
    let models = catalog(&["alpha only", "axlpxhxa beta", "alphaXXbeta"]);
    assert_eq!(
        resolve_model_query("alpha beta", &models)
            .unwrap()
            .as_deref(),
        Some("alphaXXbeta")
    );
}

#[test]
fn fallback_is_none_without_query_rewriting_or_empty_query_matching() {
    let models = catalog(&["alpha"]);
    let query = "custom/provider-z";
    assert_eq!(resolve_model_query(query, &models).unwrap(), None);
    assert_eq!(query, "custom/provider-z");
    assert_eq!(resolve_model_query("", &models).unwrap(), None);
    assert_eq!(resolve_model_query("alpha", &catalog(&[])).unwrap(), None);
}

#[test]
fn tokens_use_only_four_splitters_and_ignore_tokens_after_sixteen() {
    for query in ["alpha beta", "alpha-beta", "alpha/beta", "alpha_beta"] {
        assert_eq!(
            resolve_model_query(query, &catalog(&["alphaXXbeta"]))
                .unwrap()
                .as_deref(),
            Some("alphaXXbeta")
        );
    }
    assert_eq!(
        resolve_model_query("alpha.beta", &catalog(&["alphaXXbeta"])).unwrap(),
        None
    );
    let query = format!("{} absent-seventeenth", vec!["alpha"; 16].join(" "));
    assert_eq!(
        resolve_model_query(&query, &catalog(&["alpha"]))
            .unwrap()
            .as_deref(),
        Some("alpha")
    );
}

#[test]
fn utf8_is_opaque_and_ascii_case_folding_never_folds_unicode() {
    let models = catalog(&[
        "provider/Éclair",
        "provider/éclair",
        "\u{85}model name\u{a0}",
    ]);
    assert_eq!(
        resolve_model_query("éCLAIR", &models).unwrap().as_deref(),
        Some("provider/éclair")
    );
    assert_eq!(
        resolve_model_query("\u{85}MODEL NAME\u{a0}", &models)
            .unwrap()
            .as_deref(),
        Some("\u{85}model name\u{a0}")
    );
    assert_eq!(resolve_model_query("é", &catalog(&["É"])).unwrap(), None);
    let boundary = "é".repeat(512);
    assert_eq!(boundary.len(), machine_god_core::MAX_MODEL_ID_BYTES);
    assert_eq!(
        resolve_model_query(&boundary, &catalog(&[&boundary])).unwrap(),
        Some(boundary)
    );
}

#[test]
fn edge_spaces_are_preserved_for_scoring_not_trimmed_into_an_exact_match() {
    // Both entries receive the any-token score; trimming would select the second.
    let models = catalog(&["prefix-model", "model"]);
    assert_eq!(
        resolve_model_query(" model ", &models).unwrap().as_deref(),
        Some("prefix-model")
    );
    assert_eq!(resolve_model_query("   ", &models).unwrap(), None);
}

#[test]
fn matching_accepts_routed_long_and_control_queries_before_fallback_id_validation() {
    let models = catalog(&["model"]);
    let exact = format!("model {}", "x".repeat(MAX_NATIVE_MODEL_QUERY_BYTES - 6));
    assert_eq!(resolve_model_query(&exact, &models).unwrap().as_deref(), Some("model"));
    assert_eq!(
        resolve_model_query(&("é".repeat(MAX_NATIVE_MODEL_QUERY_BYTES / 2) + "x"), &models),
        Err(NativeModelSelectionError::QueryTooLong)
    );
    for byte in (0_u8..=31).chain([127]) {
        let query = format!("PRIVATE_QUERY{}end model", char::from(byte));
        assert_eq!(resolve_model_query(&query, &models).unwrap().as_deref(), Some("model"));
        assert!(machine_god_core::validate_model_id(&query).is_err());
    }
}

#[test]
fn catalog_limits_precede_early_exact_matches_and_empty_queries() {
    let exact_count = catalog(&vec!["model"; 512]);
    assert!(
        resolve_model_query("model", &exact_count)
            .unwrap()
            .is_some()
    );
    let excess_count = catalog(&vec!["model"; 513]);
    assert_eq!(
        resolve_model_query("model", &excess_count),
        Err(NativeModelSelectionError::CatalogLimit)
    );
    assert_eq!(
        resolve_model_query("", &excess_count),
        Err(NativeModelSelectionError::CatalogLimit)
    );
    let long = "x".repeat(1024);
    let exact_bytes = catalog(&vec![long.as_str(); 24]);
    assert_eq!(
        resolve_model_query(&long, &exact_bytes).unwrap(),
        Some(long.clone())
    );
    let mut ids = vec![long.as_str(); 24];
    ids.push("x");
    assert_eq!(
        resolve_model_query(&long, &catalog(&ids)),
        Err(NativeModelSelectionError::CatalogLimit)
    );
}
