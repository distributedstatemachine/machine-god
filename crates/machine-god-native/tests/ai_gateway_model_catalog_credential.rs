#![cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]

use std::ffi::OsString;

use machine_god_native::{
    AI_GATEWAY_HTTP_MAX_BEARER_TOKEN_BYTES, AiGatewayCredentialEnvironment,
    AiGatewayCredentialErrorKind, AiGatewayCredentialSource, DiscoveredAiGatewayCatalogCredential,
    discover_ai_gateway_catalog_credential,
};

const OIDC: &str = "oidc-catalog-token_NEVER_REAL";
const API_KEY: &str = "api-catalog-key_NEVER_REAL";

fn discover(
    oidc: Option<OsString>,
    api_key: Option<OsString>,
) -> Result<DiscoveredAiGatewayCatalogCredential, machine_god_native::AiGatewayCredentialError> {
    discover_ai_gateway_catalog_credential(AiGatewayCredentialEnvironment::new(oidc, api_key))
}

fn assert_source(
    credential: DiscoveredAiGatewayCatalogCredential,
    expected: AiGatewayCredentialSource,
) {
    let DiscoveredAiGatewayCatalogCredential::Authenticated(credential) = credential else {
        panic!("expected authenticated catalog credential")
    };
    assert_eq!(credential.source(), expected);
    let debug = format!("{credential:?}");
    assert!(!debug.contains(OIDC));
    assert!(!debug.contains(API_KEY));
    assert_eq!(
        format!("{:?}", credential.into_bearer_token()),
        "AiGatewayBearerToken(<redacted>)"
    );
}

#[test]
fn missing_and_empty_values_select_public_catalog_access() {
    for (oidc, api_key) in [
        (None, None),
        (Some(OsString::new()), None),
        (None, Some(OsString::new())),
        (Some(OsString::new()), Some(OsString::new())),
    ] {
        let credential = discover(oidc, api_key).unwrap();
        assert!(matches!(
            credential,
            DiscoveredAiGatewayCatalogCredential::PublicOnly
        ));
        assert_eq!(
            format!("{credential:?}"),
            "DiscoveredAiGatewayCatalogCredential::PublicOnly"
        );
    }
}

#[test]
fn empty_oidc_falls_through_and_nonempty_oidc_has_precedence() {
    assert_source(
        discover(Some(OsString::new()), Some(API_KEY.into())).unwrap(),
        AiGatewayCredentialSource::AiGatewayApiKey,
    );
    assert_source(
        discover(Some(OIDC.into()), Some(API_KEY.into())).unwrap(),
        AiGatewayCredentialSource::VercelOidcToken,
    );
    assert_source(
        discover(None, Some(API_KEY.into())).unwrap(),
        AiGatewayCredentialSource::AiGatewayApiKey,
    );
}

#[test]
fn selected_invalid_or_oversized_values_fail_closed_without_fallthrough() {
    for invalid_oidc in [
        "contains space".to_owned(),
        "line\r\ninjection".to_owned(),
        "=leading-padding".to_owned(),
        "padding=then-data".to_owned(),
        "non-ascii-é".to_owned(),
        "x".repeat(AI_GATEWAY_HTTP_MAX_BEARER_TOKEN_BYTES + 1),
    ] {
        let error = discover(Some(invalid_oidc.into()), Some(API_KEY.into())).unwrap_err();
        assert_eq!(
            error.kind(),
            AiGatewayCredentialErrorKind::InvalidBearerToken
        );
        let diagnostic = format!("{error:?} {error}");
        assert!(!diagnostic.contains(API_KEY));
        assert!(!diagnostic.contains("line\r\ninjection"));
    }

    let error = discover(
        Some(OsString::new()),
        Some(
            "x".repeat(AI_GATEWAY_HTTP_MAX_BEARER_TOKEN_BYTES + 1)
                .into(),
        ),
    )
    .unwrap_err();
    assert_eq!(
        error.kind(),
        AiGatewayCredentialErrorKind::InvalidBearerToken
    );
}

#[test]
fn exact_bearer_bound_and_catalog_debug_are_redacted() {
    let credential = discover(
        None,
        Some("x".repeat(AI_GATEWAY_HTTP_MAX_BEARER_TOKEN_BYTES).into()),
    )
    .unwrap();
    let debug = format!("{credential:?}");
    assert!(debug.contains("Authenticated"));
    assert!(debug.contains("AiGatewayApiKey"));
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains(&"x".repeat(32)));

    let snapshot = AiGatewayCredentialEnvironment::new(
        Some("SNAPSHOT_SECRET_MARKER".into()),
        Some("LOWER_SECRET_MARKER".into()),
    );
    assert_eq!(
        format!("{snapshot:?}"),
        "AiGatewayCredentialEnvironment(<redacted>)"
    );
    let credential = discover_ai_gateway_catalog_credential(snapshot).unwrap();
    let debug = format!("{credential:?}");
    assert!(!debug.contains("SNAPSHOT_SECRET_MARKER"));
    assert!(!debug.contains("LOWER_SECRET_MARKER"));
}

#[test]
fn credential_errors_have_closed_redacted_debug_and_display() {
    let error = discover(
        Some("HOSTILE_SECRET\r\nMARKER".into()),
        Some(API_KEY.into()),
    )
    .unwrap_err();
    assert_eq!(
        error.kind(),
        AiGatewayCredentialErrorKind::InvalidBearerToken
    );
    assert_eq!(error.to_string(), "AI Gateway bearer token is invalid");
    assert_eq!(
        format!("{error:?}"),
        "AiGatewayCredentialError { kind: InvalidBearerToken }"
    );
    let diagnostic = format!("{error:?} {error}");
    assert!(!diagnostic.contains("HOSTILE_SECRET"));
    assert!(!diagnostic.contains("MARKER"));
    assert!(!diagnostic.contains(API_KEY));
}

#[cfg(unix)]
#[test]
fn selected_non_unicode_values_fail_closed_and_unselected_lower_values_do_not() {
    use std::os::unix::ffi::OsStringExt;

    let mut invalid_oidc = b"NON_UNICODE_OIDC_SECRET".to_vec();
    invalid_oidc.push(0xff);
    let error = discover(Some(OsString::from_vec(invalid_oidc)), Some(API_KEY.into())).unwrap_err();
    assert_eq!(
        error.kind(),
        AiGatewayCredentialErrorKind::InvalidEnvironment
    );
    assert_eq!(
        error.to_string(),
        "AI Gateway credential environment is invalid"
    );
    assert!(!format!("{error:?} {error}").contains("NON_UNICODE_OIDC_SECRET"));

    let mut invalid_api_key = b"NON_UNICODE_API_SECRET".to_vec();
    invalid_api_key.push(0xff);
    let error = discover(
        Some(OsString::new()),
        Some(OsString::from_vec(invalid_api_key.clone())),
    )
    .unwrap_err();
    assert_eq!(
        error.kind(),
        AiGatewayCredentialErrorKind::InvalidEnvironment
    );
    assert!(!format!("{error:?} {error}").contains("NON_UNICODE_API_SECRET"));

    assert_source(
        discover(Some(OIDC.into()), Some(OsString::from_vec(invalid_api_key))).unwrap(),
        AiGatewayCredentialSource::VercelOidcToken,
    );
}
