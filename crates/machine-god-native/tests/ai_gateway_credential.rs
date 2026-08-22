#![cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]

use std::error::Error;
use std::ffi::OsString;
use std::sync::{Arc, Barrier};
use std::thread;

use machine_god_native::{
    AI_GATEWAY_API_KEY_ENV, AI_GATEWAY_HTTP_MAX_BEARER_TOKEN_BYTES, AiGatewayCredentialEnvironment,
    AiGatewayCredentialError, AiGatewayCredentialErrorKind, AiGatewayCredentialSource,
    DiscoveredAiGatewayCredential, VERCEL_OIDC_TOKEN_ENV, discover_ai_gateway_credential,
};

const OIDC_TOKEN: &str = "oidc-token_NEVER_REAL";
const API_KEY: &str = "api-key_NEVER_REAL";

fn environment(
    oidc: Option<OsString>,
    api_key: Option<OsString>,
) -> AiGatewayCredentialEnvironment {
    AiGatewayCredentialEnvironment::new(oidc, api_key)
}

fn discover(
    oidc: Option<impl Into<OsString>>,
    api_key: Option<impl Into<OsString>>,
) -> Result<DiscoveredAiGatewayCredential, AiGatewayCredentialError> {
    discover_ai_gateway_credential(environment(oidc.map(Into::into), api_key.map(Into::into)))
}

fn discovery_error(
    oidc: Option<impl Into<OsString>>,
    api_key: Option<impl Into<OsString>>,
) -> AiGatewayCredentialError {
    discover(oidc, api_key).unwrap_err()
}

fn assert_error(
    error: AiGatewayCredentialError,
    expected_kind: AiGatewayCredentialErrorKind,
    expected_display: &str,
) {
    assert_eq!(error.kind(), expected_kind);
    assert_eq!(error.to_string(), expected_display);
    assert_eq!(
        format!("{error:?}"),
        format!("AiGatewayCredentialError {{ kind: {expected_kind:?} }}")
    );
    assert!(error.source().is_none());
}

#[test]
fn public_names_sources_and_missing_diagnostics_are_stable() {
    assert_eq!(VERCEL_OIDC_TOKEN_ENV, "VERCEL_OIDC_TOKEN");
    assert_eq!(AI_GATEWAY_API_KEY_ENV, "AI_GATEWAY_API_KEY");
    assert_eq!(
        AiGatewayCredentialSource::VercelOidcToken.as_str(),
        "vercel_oidc_token"
    );
    assert_eq!(
        AiGatewayCredentialSource::AiGatewayApiKey.as_str(),
        "ai_gateway_api_key"
    );

    let error = discovery_error(None::<OsString>, None::<OsString>);
    assert_error(
        error,
        AiGatewayCredentialErrorKind::Missing,
        "AI Gateway credential is missing",
    );
}

#[test]
fn empty_values_are_absent_and_oidc_precedes_the_api_key() {
    for (oidc, api_key) in [
        (None, None),
        (Some(OsString::new()), None),
        (None, Some(OsString::new())),
        (Some(OsString::new()), Some(OsString::new())),
    ] {
        let error = discover_ai_gateway_credential(environment(oidc, api_key)).unwrap_err();
        assert_eq!(error.kind(), AiGatewayCredentialErrorKind::Missing);
    }

    let api_key = discover(Some(OsString::new()), Some(API_KEY)).unwrap();
    assert_eq!(api_key.source(), AiGatewayCredentialSource::AiGatewayApiKey);

    let oidc = discover(Some(OIDC_TOKEN), Some(API_KEY)).unwrap();
    assert_eq!(oidc.source(), AiGatewayCredentialSource::VercelOidcToken);
}

#[test]
fn selected_invalid_values_fail_closed_and_unselected_values_are_ignored() {
    for invalid_oidc in [
        " ".to_owned(),
        "line\r\ninjection".to_owned(),
        "=leading-padding".to_owned(),
        "padding=then-data".to_owned(),
        "not-ascii-é".to_owned(),
        "x".repeat(AI_GATEWAY_HTTP_MAX_BEARER_TOKEN_BYTES + 1),
    ] {
        let error = discovery_error(Some(invalid_oidc), Some(API_KEY));
        assert_eq!(
            error.kind(),
            AiGatewayCredentialErrorKind::InvalidBearerToken
        );
    }

    let selected = discover(Some(OIDC_TOKEN), Some("invalid lower source")).unwrap();
    assert_eq!(
        selected.source(),
        AiGatewayCredentialSource::VercelOidcToken
    );
}

#[test]
fn bearer_grammar_and_exact_byte_bound_are_preserved() {
    for accepted in [
        "a".to_owned(),
        "Az09-._~+/".to_owned(),
        "abc+/._~-==".to_owned(),
        "a".repeat(AI_GATEWAY_HTTP_MAX_BEARER_TOKEN_BYTES),
    ] {
        let credential = discover(None::<OsString>, Some(accepted)).unwrap();
        assert_eq!(
            credential.source(),
            AiGatewayCredentialSource::AiGatewayApiKey
        );
    }

    for rejected in [
        " ".to_owned(),
        "\t".to_owned(),
        "\r".to_owned(),
        "\n".to_owned(),
        "token with space".to_owned(),
        "token\0tail".to_owned(),
        "token\u{7f}".to_owned(),
        "token\u{1b}[31m".to_owned(),
        "=token".to_owned(),
        "tok=en".to_owned(),
        "token=tail".to_owned(),
        "token-é".to_owned(),
        "a".repeat(AI_GATEWAY_HTTP_MAX_BEARER_TOKEN_BYTES + 1),
    ] {
        let error = discovery_error(None::<OsString>, Some(rejected));
        assert_eq!(
            error.kind(),
            AiGatewayCredentialErrorKind::InvalidBearerToken
        );
    }
}

#[test]
fn snapshots_credentials_tokens_and_errors_are_redacted() {
    let snapshot = environment(
        Some(OsString::from("OIDC_SECRET_MARKER")),
        Some(OsString::from("API_SECRET_MARKER")),
    );
    assert_eq!(
        format!("{snapshot:?}"),
        "AiGatewayCredentialEnvironment(<redacted>)"
    );

    let credential = discover_ai_gateway_credential(snapshot).unwrap();
    let credential_debug = format!("{credential:?}");
    assert_eq!(
        credential_debug,
        "DiscoveredAiGatewayCredential { source: VercelOidcToken, bearer_token: \"<redacted>\" }"
    );
    for forbidden in ["OIDC_SECRET_MARKER", "API_SECRET_MARKER", "SECRET_MARKER"] {
        assert!(!credential_debug.contains(forbidden));
    }
    let token = credential.into_bearer_token();
    assert_eq!(format!("{token:?}"), "AiGatewayBearerToken(<redacted>)");

    let invalid = "LEFT_SECRET_MARKER\r\nRIGHT_SECRET_MARKER";
    let error = discovery_error(Some(invalid), Some(API_KEY));
    assert_error(
        error,
        AiGatewayCredentialErrorKind::InvalidBearerToken,
        "AI Gateway bearer token is invalid",
    );
    let diagnostics = format!("{error:?} {error}");
    for forbidden in [
        invalid,
        "LEFT_SECRET_MARKER",
        "RIGHT_SECRET_MARKER",
        "SECRET_MARKER",
    ] {
        assert!(!diagnostics.contains(forbidden));
    }

    let oversized = format!(
        "OVERSIZED_LEFT_MARKER{}OVERSIZED_RIGHT_MARKER",
        "x".repeat(AI_GATEWAY_HTTP_MAX_BEARER_TOKEN_BYTES)
    );
    let error = discovery_error(None::<OsString>, Some(oversized));
    let diagnostics = format!("{error:?} {error}");
    for forbidden in ["OVERSIZED_LEFT_MARKER", "OVERSIZED_RIGHT_MARKER"] {
        assert!(!diagnostics.contains(forbidden));
    }
}

#[test]
fn snapshot_owns_and_prevalidates_its_injected_values() {
    let mut caller_copy = OsString::from(OIDC_TOKEN);
    let snapshot = environment(Some(caller_copy.clone()), None);
    caller_copy.clear();

    let credential = discover_ai_gateway_credential(snapshot).unwrap();
    assert_eq!(
        credential.source(),
        AiGatewayCredentialSource::VercelOidcToken
    );
}

#[cfg(unix)]
#[test]
fn selected_non_unicode_unix_values_fail_closed_and_are_redacted() {
    use std::os::unix::ffi::OsStringExt;

    let mut invalid = b"NON_UNICODE_SECRET_MARKER".to_vec();
    invalid.push(0xff);
    let error = discover_ai_gateway_credential(environment(
        Some(OsString::from_vec(invalid)),
        Some(OsString::from(API_KEY)),
    ))
    .unwrap_err();
    assert_error(
        error,
        AiGatewayCredentialErrorKind::InvalidEnvironment,
        "AI Gateway credential environment is invalid",
    );
    let diagnostics = format!("{error:?} {error}");
    assert!(!diagnostics.contains("NON_UNICODE_SECRET_MARKER"));

    let mut ignored = b"IGNORED_NON_UNICODE_MARKER".to_vec();
    ignored.push(0xff);
    let credential = discover_ai_gateway_credential(environment(
        Some(OsString::from(OIDC_TOKEN)),
        Some(OsString::from_vec(ignored)),
    ))
    .unwrap();
    assert_eq!(
        credential.source(),
        AiGatewayCredentialSource::VercelOidcToken
    );

    let mut invalid_api_key = b"NON_UNICODE_API_KEY_MARKER".to_vec();
    invalid_api_key.push(0xff);
    let error = discover_ai_gateway_credential(environment(
        Some(OsString::new()),
        Some(OsString::from_vec(invalid_api_key)),
    ))
    .unwrap_err();
    assert_error(
        error,
        AiGatewayCredentialErrorKind::InvalidEnvironment,
        "AI Gateway credential environment is invalid",
    );
    assert!(!format!("{error:?} {error}").contains("NON_UNICODE_API_KEY_MARKER"));
}

#[cfg(windows)]
#[test]
fn selected_non_unicode_windows_values_fail_closed_and_are_redacted() {
    use std::os::windows::ffi::OsStringExt;

    let marker: Vec<u16> = "NON_UNICODE_SECRET_MARKER"
        .encode_utf16()
        .chain([0xd800])
        .collect();
    let error = discover_ai_gateway_credential(environment(
        Some(OsString::from_wide(&marker)),
        Some(OsString::from(API_KEY)),
    ))
    .unwrap_err();
    assert_error(
        error,
        AiGatewayCredentialErrorKind::InvalidEnvironment,
        "AI Gateway credential environment is invalid",
    );
    assert!(!format!("{error:?} {error}").contains("NON_UNICODE_SECRET_MARKER"));

    let api_key_marker: Vec<u16> = "NON_UNICODE_API_KEY_MARKER"
        .encode_utf16()
        .chain([0xd800])
        .collect();
    let error = discover_ai_gateway_credential(environment(
        Some(OsString::new()),
        Some(OsString::from_wide(&api_key_marker)),
    ))
    .unwrap_err();
    assert_error(
        error,
        AiGatewayCredentialErrorKind::InvalidEnvironment,
        "AI Gateway credential environment is invalid",
    );
    assert!(!format!("{error:?} {error}").contains("NON_UNICODE_API_KEY_MARKER"));
}

#[test]
fn synchronized_independent_snapshots_do_not_share_resolution_state() {
    const THREADS: usize = 24;

    fn assert_send<T: Send>() {}
    assert_send::<AiGatewayCredentialEnvironment>();
    assert_send::<DiscoveredAiGatewayCredential>();

    let barrier = Arc::new(Barrier::new(THREADS));
    let workers = (0..THREADS)
        .map(|index| {
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let snapshot = match index % 3 {
                    0 => environment(Some(OsString::from(OIDC_TOKEN)), None),
                    1 => environment(None, Some(OsString::from(API_KEY))),
                    _ => environment(Some(OsString::from("invalid selected value")), None),
                };
                barrier.wait();
                match index % 3 {
                    0 => assert_eq!(
                        discover_ai_gateway_credential(snapshot).unwrap().source(),
                        AiGatewayCredentialSource::VercelOidcToken
                    ),
                    1 => assert_eq!(
                        discover_ai_gateway_credential(snapshot).unwrap().source(),
                        AiGatewayCredentialSource::AiGatewayApiKey
                    ),
                    _ => assert_eq!(
                        discover_ai_gateway_credential(snapshot).unwrap_err().kind(),
                        AiGatewayCredentialErrorKind::InvalidBearerToken
                    ),
                }
            })
        })
        .collect::<Vec<_>>();

    for worker in workers {
        worker.join().expect("credential worker panicked");
    }
}
