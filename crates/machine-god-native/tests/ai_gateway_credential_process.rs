#![cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]

use std::ffi::OsString;
use std::process::{Command, Output};

use machine_god_native::{
    AI_GATEWAY_API_KEY_ENV, AI_GATEWAY_HTTP_MAX_BEARER_TOKEN_BYTES, AiGatewayCredentialErrorKind,
    AiGatewayCredentialSource, VERCEL_OIDC_TOKEN_ENV, discover_process_ai_gateway_credential,
};

const CHILD_MODE_ENV: &str = "MACHINE_GOD_AI_GATEWAY_CREDENTIAL_TEST_CHILD_MODE";
const OIDC_TOKEN: &str = "process-oidc_NEVER_REAL";
const API_KEY: &str = "process-api-key_NEVER_REAL";
const INVALID_MARKER: &str = "PROCESS_INVALID_SECRET_MARKER";

#[derive(Clone, Copy)]
enum Expected {
    Source(AiGatewayCredentialSource),
    Error(AiGatewayCredentialErrorKind),
}

struct ProcessCase {
    mode: &'static str,
    oidc: Option<OsString>,
    api_key: Option<OsString>,
    expected: Expected,
}

fn run_child(case: ProcessCase) -> Output {
    let mut command = Command::new(std::env::current_exe().expect("credential test executable"));
    command
        .arg("--exact")
        .arg("ambient_process_discovery_probe")
        .arg("--nocapture")
        .env(CHILD_MODE_ENV, case.mode)
        .env_remove(VERCEL_OIDC_TOKEN_ENV)
        .env_remove(AI_GATEWAY_API_KEY_ENV);
    if let Some(value) = case.oidc {
        command.env(VERCEL_OIDC_TOKEN_ENV, value);
    }
    if let Some(value) = case.api_key {
        command.env(AI_GATEWAY_API_KEY_ENV, value);
    }

    let expected_code = match case.expected {
        Expected::Source(AiGatewayCredentialSource::VercelOidcToken) => "source-oidc",
        Expected::Source(AiGatewayCredentialSource::AiGatewayApiKey) => "source-api-key",
        Expected::Source(_) => panic!("unsupported non-exhaustive credential source fixture"),
        Expected::Error(AiGatewayCredentialErrorKind::Missing) => "error-missing",
        Expected::Error(AiGatewayCredentialErrorKind::InvalidEnvironment) => {
            "error-invalid-environment"
        }
        Expected::Error(AiGatewayCredentialErrorKind::InvalidBearerToken) => {
            "error-invalid-bearer-token"
        }
        Expected::Error(_) => panic!("unsupported non-exhaustive credential error fixture"),
    };
    command.env(
        "MACHINE_GOD_AI_GATEWAY_CREDENTIAL_TEST_EXPECTED",
        expected_code,
    );
    command.output().expect("run credential discovery child")
}

fn assert_child_success(mode: &str, output: &Output) {
    assert!(
        output.status.success(),
        "credential child failed for mode {mode}; stdout bytes={}, stderr bytes={}",
        output.stdout.len(),
        output.stderr.len()
    );
    for bytes in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(bytes);
        for forbidden in [
            OIDC_TOKEN,
            API_KEY,
            INVALID_MARKER,
            "PROCESS_OVERSIZED_LEFT_MARKER",
            "PROCESS_OVERSIZED_RIGHT_MARKER",
            "PROCESS_NON_UNICODE_SECRET_MARKER",
        ] {
            assert!(
                !text.contains(forbidden),
                "credential child output leaked a synthetic secret marker in mode {mode}"
            );
        }
    }
}

#[test]
fn ambient_process_discovery_probe() {
    let Some(mode) = std::env::var_os(CHILD_MODE_ENV) else {
        return;
    };
    let mode = mode
        .into_string()
        .expect("credential child mode must be Unicode");
    let expected = std::env::var("MACHINE_GOD_AI_GATEWAY_CREDENTIAL_TEST_EXPECTED")
        .expect("credential child expected result");
    let result = discover_process_ai_gateway_credential();

    match expected.as_str() {
        "source-oidc" => assert_eq!(
            result.expect("expected OIDC credential").source(),
            AiGatewayCredentialSource::VercelOidcToken,
            "child mode {mode}"
        ),
        "source-api-key" => assert_eq!(
            result.expect("expected API-key credential").source(),
            AiGatewayCredentialSource::AiGatewayApiKey,
            "child mode {mode}"
        ),
        "error-missing" => assert_eq!(
            result.expect_err("expected missing credential").kind(),
            AiGatewayCredentialErrorKind::Missing,
            "child mode {mode}"
        ),
        "error-invalid-environment" => assert_eq!(
            result
                .expect_err("expected invalid credential environment")
                .kind(),
            AiGatewayCredentialErrorKind::InvalidEnvironment,
            "child mode {mode}"
        ),
        "error-invalid-bearer-token" => assert_eq!(
            result.expect_err("expected invalid bearer token").kind(),
            AiGatewayCredentialErrorKind::InvalidBearerToken,
            "child mode {mode}"
        ),
        other => panic!("unsupported credential child expectation {other}"),
    }
}

#[test]
fn ambient_discovery_isolated_in_subprocesses_without_parent_environment_mutation() {
    let oversized = format!(
        "PROCESS_OVERSIZED_LEFT_MARKER{}PROCESS_OVERSIZED_RIGHT_MARKER",
        "x".repeat(AI_GATEWAY_HTTP_MAX_BEARER_TOKEN_BYTES)
    );
    let cases = [
        ProcessCase {
            mode: "missing",
            oidc: None,
            api_key: None,
            expected: Expected::Error(AiGatewayCredentialErrorKind::Missing),
        },
        ProcessCase {
            mode: "both-empty",
            oidc: Some(OsString::new()),
            api_key: Some(OsString::new()),
            expected: Expected::Error(AiGatewayCredentialErrorKind::Missing),
        },
        ProcessCase {
            mode: "api-key",
            oidc: None,
            api_key: Some(OsString::from(API_KEY)),
            expected: Expected::Source(AiGatewayCredentialSource::AiGatewayApiKey),
        },
        ProcessCase {
            mode: "empty-oidc-fallback",
            oidc: Some(OsString::new()),
            api_key: Some(OsString::from(API_KEY)),
            expected: Expected::Source(AiGatewayCredentialSource::AiGatewayApiKey),
        },
        ProcessCase {
            mode: "oidc-precedence",
            oidc: Some(OsString::from(OIDC_TOKEN)),
            api_key: Some(OsString::from(API_KEY)),
            expected: Expected::Source(AiGatewayCredentialSource::VercelOidcToken),
        },
        ProcessCase {
            mode: "invalid-oidc-fail-closed",
            oidc: Some(OsString::from(format!("{INVALID_MARKER} with space"))),
            api_key: Some(OsString::from(API_KEY)),
            expected: Expected::Error(AiGatewayCredentialErrorKind::InvalidBearerToken),
        },
        ProcessCase {
            mode: "oversized-oidc-fail-closed",
            oidc: Some(OsString::from(oversized)),
            api_key: Some(OsString::from(API_KEY)),
            expected: Expected::Error(AiGatewayCredentialErrorKind::InvalidBearerToken),
        },
    ];

    for case in cases {
        let mode = case.mode;
        let output = run_child(case);
        assert_child_success(mode, &output);
    }
}

#[cfg(unix)]
#[test]
fn ambient_non_unicode_source_fails_closed_in_an_isolated_subprocess() {
    use std::os::unix::ffi::OsStringExt;

    let mut invalid = b"PROCESS_NON_UNICODE_SECRET_MARKER".to_vec();
    invalid.push(0xff);
    let case = ProcessCase {
        mode: "non-unicode-oidc-fail-closed",
        oidc: Some(OsString::from_vec(invalid)),
        api_key: Some(OsString::from(API_KEY)),
        expected: Expected::Error(AiGatewayCredentialErrorKind::InvalidEnvironment),
    };
    let output = run_child(case);
    assert_child_success("non-unicode-oidc-fail-closed", &output);
}
