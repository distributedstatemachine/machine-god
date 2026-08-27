use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use machine_god_native::{
    AI_GATEWAY_DEFAULT_MODEL, CONFIG_SCHEMA_VERSION, ConfigOrigin, LoadedNativeConfig,
    MAX_CONFIG_BYTES, NativeConfigError, NativeConfigErrorKind, NativeCredentialSourceKind,
    NativeEnvironment, NativeProviderKind, NativeTransportKind, PermissionMode, load_native_config,
};

#[cfg(all(
    feature = "ai-gateway-http",
    not(target_family = "wasm"),
    any(target_os = "linux", target_os = "macos")
))]
mod web_search_support;

#[cfg(all(
    feature = "ai-gateway-http",
    not(target_family = "wasm"),
    any(target_os = "linux", target_os = "macos")
))]
use web_search_support::{never_deadline, production_gateway_target};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-configured-credential-{label}-{}-{identifier}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("failed to create a temporary directory: {error}"),
            }
        }
        panic!("failed to allocate a unique temporary directory");
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let result = fs::remove_dir_all(&self.path);
        if std::thread::panicking() {
            return;
        }
        match result {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove a temporary directory: {error}"),
        }
    }
}

fn environment(config_root: &Path) -> NativeEnvironment {
    NativeEnvironment::new(Some(OsString::from(config_root.as_os_str())), None, None)
}

fn config_path(config_root: &Path) -> PathBuf {
    config_root.join("machine-god").join("config.json")
}

fn write_config(config_root: &Path, contents: &[u8]) -> PathBuf {
    let path = config_path(config_root);
    fs::create_dir_all(path.parent().expect("configuration path has a parent")).unwrap();
    fs::write(&path, contents).unwrap();
    path
}

fn valid_v1() -> &'static str {
    r#"{"schema_version":1,"permission_mode":"ask"}"#
}

fn valid_v2(model: &str) -> String {
    format!(
        r#"{{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"{model}"}}"#
    )
}

fn valid_v3(model: &str) -> String {
    format!(
        r#"{{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"{model}","credential_source":"environment"}}"#
    )
}

fn raw_v3(
    schema_version: &str,
    permission_mode: &str,
    provider: &str,
    transport: &str,
    model: &str,
    credential_source: &str,
) -> String {
    format!(
        r#"{{"schema_version":{schema_version},"permission_mode":{permission_mode},"provider":{provider},"transport":{transport},"model":{model},"credential_source":{credential_source}}}"#
    )
}

fn load(config_root: &Path, contents: &[u8]) -> LoadedNativeConfig {
    write_config(config_root, contents);
    load_native_config(&environment(config_root)).unwrap()
}

fn load_error(config_root: &Path, contents: &[u8]) -> NativeConfigError {
    write_config(config_root, contents);
    load_native_config(&environment(config_root)).unwrap_err()
}

fn assert_loaded(
    loaded: &LoadedNativeConfig,
    origin: ConfigOrigin,
    schema_version: u32,
    model: &str,
) {
    assert_eq!(loaded.origin(), origin);
    let config = loaded.config();
    assert_eq!(config.schema_version(), schema_version);
    assert_eq!(config.permission_mode(), PermissionMode::Ask);
    assert_eq!(config.provider(), NativeProviderKind::VercelAiGateway);
    assert_eq!(config.transport(), NativeTransportKind::AiGatewayHttp);
    assert_eq!(config.model(), model);
    assert_eq!(
        config.credential_source(),
        NativeCredentialSourceKind::Environment
    );
}

fn assert_format_error(config_root: &Path, contents: &str) {
    let error = load_error(config_root, contents.as_bytes());
    assert_eq!(error.kind(), NativeConfigErrorKind::InvalidFormat);
    assert_eq!(error.to_string(), "native configuration format is invalid");
}

#[test]
fn schema_v3_public_contract_and_built_in_projection_are_stable() {
    assert_eq!(CONFIG_SCHEMA_VERSION, 3);
    assert_eq!(
        NativeCredentialSourceKind::Environment.as_str(),
        "environment"
    );

    let temporary = TemporaryDirectory::new("built-in");
    let absent_root = temporary.path().join("absent");
    let loaded = load_native_config(&environment(&absent_root)).unwrap();
    assert_loaded(
        &loaded,
        ConfigOrigin::BuiltInDefaults,
        3,
        AI_GATEWAY_DEFAULT_MODEL,
    );
    assert!(!absent_root.exists());
}

#[test]
fn exact_schema_v3_loads_without_rewriting_file_bytes() {
    let temporary = TemporaryDirectory::new("v3");
    let config_root = temporary.path().join("xdg");
    let contents = valid_v3("provider/custom-model").into_bytes();
    let path = write_config(&config_root, &contents);

    let loaded = load_native_config(&environment(&config_root)).unwrap();

    assert_loaded(&loaded, ConfigOrigin::File, 3, "provider/custom-model");
    assert_eq!(fs::read(path).unwrap(), contents);
}

#[test]
fn legacy_v1_and_v2_remain_observable_and_project_environment_without_rewriting() {
    for (label, schema_version, contents, model) in [
        ("v1", 1, valid_v1().to_owned(), AI_GATEWAY_DEFAULT_MODEL),
        (
            "v2",
            2,
            valid_v2("provider/legacy-v2-model"),
            "provider/legacy-v2-model",
        ),
    ] {
        let temporary = TemporaryDirectory::new(label);
        let config_root = temporary.path().join("xdg");
        let original = contents.into_bytes();
        let path = write_config(&config_root, &original);

        let loaded = load_native_config(&environment(&config_root)).unwrap();

        assert_loaded(&loaded, ConfigOrigin::File, schema_version, model);
        assert_eq!(fs::read(path).unwrap(), original);
    }
}

#[test]
fn schema_v2_does_not_accept_the_schema_v3_field() {
    let temporary = TemporaryDirectory::new("v2-extra");
    let config_root = temporary.path().join("xdg");
    let contents = r#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":"environment"}"#;

    assert_format_error(&config_root, contents);
}

#[test]
fn schema_v3_requires_exactly_six_fields() {
    let temporary = TemporaryDirectory::new("v3-shape");
    let config_root = temporary.path().join("xdg");
    let cases = [
        r#"{"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":"environment"}"#,
        r#"{"schema_version":3,"provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":"environment"}"#,
        r#"{"schema_version":3,"permission_mode":"ask","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":"environment"}"#,
        r#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","model":"zai/glm-5.2","credential_source":"environment"}"#,
        r#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","credential_source":"environment"}"#,
        r#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2"}"#,
        r#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":"environment","unexpected":true}"#,
    ];

    for contents in cases {
        assert_format_error(&config_root, contents);
    }
}

#[test]
fn schema_v3_rejects_every_duplicate_field() {
    let temporary = TemporaryDirectory::new("v3-duplicates");
    let config_root = temporary.path().join("xdg");
    let cases = [
        r#"{"schema_version":3,"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":"environment"}"#,
        r#"{"schema_version":3,"permission_mode":"ask","permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":"environment"}"#,
        r#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":"environment"}"#,
        r#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":"environment"}"#,
        r#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","model":"zai/glm-5.2","credential_source":"environment"}"#,
        r#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":"environment","credential_source":"environment"}"#,
    ];

    for contents in cases {
        assert_format_error(&config_root, contents);
    }
}

#[test]
fn schema_v3_rejects_wrong_types_for_each_field() {
    let temporary = TemporaryDirectory::new("v3-types");
    let config_root = temporary.path().join("xdg");
    let cases = [
        raw_v3(
            r#""3""#,
            r#""ask""#,
            r#""vercel_ai_gateway""#,
            r#""ai_gateway_http""#,
            r#""zai/glm-5.2""#,
            r#""environment""#,
        ),
        raw_v3(
            "3",
            "true",
            r#""vercel_ai_gateway""#,
            r#""ai_gateway_http""#,
            r#""zai/glm-5.2""#,
            r#""environment""#,
        ),
        raw_v3(
            "3",
            r#""ask""#,
            "[]",
            r#""ai_gateway_http""#,
            r#""zai/glm-5.2""#,
            r#""environment""#,
        ),
        raw_v3(
            "3",
            r#""ask""#,
            r#""vercel_ai_gateway""#,
            "null",
            r#""zai/glm-5.2""#,
            r#""environment""#,
        ),
        raw_v3(
            "3",
            r#""ask""#,
            r#""vercel_ai_gateway""#,
            r#""ai_gateway_http""#,
            r#"{"model":true}"#,
            r#""environment""#,
        ),
        raw_v3(
            "3",
            r#""ask""#,
            r#""vercel_ai_gateway""#,
            r#""ai_gateway_http""#,
            r#""zai/glm-5.2""#,
            "1",
        ),
    ];

    for contents in cases {
        assert_format_error(&config_root, &contents);
    }
}

#[test]
fn credential_source_rejects_wrong_types_and_noncanonical_values() {
    let temporary = TemporaryDirectory::new("credential-values");
    let config_root = temporary.path().join("xdg");
    let values = [
        "null",
        "true",
        "1",
        "[]",
        r#"{"environment":true}"#,
        r#"""#,
        r#""Environment""#,
        r#"" environment""#,
        r#""environment ""#,
        r#""environmént""#,
        r#""env""#,
        r#""process_environment""#,
        r#""VERCEL_OIDC_TOKEN""#,
        r#""AI_GATEWAY_API_KEY""#,
    ];

    for value in values {
        let contents = format!(
            r#"{{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":{value}}}"#
        );
        assert_format_error(&config_root, &contents);
    }
}

#[test]
fn arbitrary_environment_names_and_secret_bearing_fields_are_rejected_and_redacted() {
    let temporary = TemporaryDirectory::new("secret-fields");
    let config_root = temporary.path().join("xdg");
    let secret = "NEVER_REFLECT_CONFIGURED_CREDENTIAL_SECRET";
    let cases = [
        format!(
            r#"{{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":"environment","environment_variable":"{secret}"}}"#
        ),
        format!(
            r#"{{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":"environment","token":"{secret}"}}"#
        ),
        format!(
            r#"{{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":"environment","api_key":"{secret}"}}"#
        ),
        format!(
            r#"{{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":"environment","credential":"{secret}"}}"#
        ),
    ];

    for contents in cases {
        let error = load_error(&config_root, contents.as_bytes());
        assert_eq!(error.kind(), NativeConfigErrorKind::InvalidFormat);
        let diagnostics = format!("{error:?} {error}");
        assert!(!diagnostics.contains(secret));
        assert!(!diagnostics.contains("environment_variable"));
        assert!(!diagnostics.contains("token"));
        assert!(!diagnostics.contains("api_key"));
    }
}

#[test]
fn future_integer_version_is_unsupported_while_malformed_versions_are_invalid() {
    let temporary = TemporaryDirectory::new("future-version");
    let config_root = temporary.path().join("xdg");
    for contents in [
        r#"{"schema_version":4}"#,
        r#"{"schema_version":4,"credential_source":"future","secret":"HIDDEN"}"#,
        r#"{"schema_version":18446744073709551616,"future":true}"#,
        r#"{"schema_version":-1,"future":true}"#,
    ] {
        let error = load_error(&config_root, contents.as_bytes());
        assert_eq!(
            error.kind(),
            NativeConfigErrorKind::UnsupportedSchemaVersion
        );
    }

    for contents in [
        r#"{"schema_version":"4"}"#,
        r#"{"schema_version":4.0}"#,
        r#"{"schema_version":true}"#,
        r#"{"schema_version":null}"#,
        r#"{"schema_version":4"#,
    ] {
        assert_format_error(&config_root, contents);
    }
}

#[test]
fn schema_v3_preserves_the_exact_file_bound_and_redacted_diagnostics() {
    let temporary = TemporaryDirectory::new("bounds");
    let config_root = temporary.path().join("xdg");
    let mut exact = valid_v3(AI_GATEWAY_DEFAULT_MODEL).into_bytes();
    exact.resize(MAX_CONFIG_BYTES, b' ');
    let loaded = load(&config_root, &exact);
    assert_loaded(&loaded, ConfigOrigin::File, 3, AI_GATEWAY_DEFAULT_MODEL);

    exact.push(b' ');
    let oversized = load_error(&config_root, &exact);
    assert_eq!(oversized.kind(), NativeConfigErrorKind::TooLarge);

    let secret = "MODEL_SECRET_MARKER";
    let invalid = format!(
        r#"{{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"{secret} with-space","credential_source":"environment"}}"#
    );
    let error = load_error(&config_root, invalid.as_bytes());
    assert_eq!(error.kind(), NativeConfigErrorKind::InvalidFormat);
    let diagnostics = format!("{error:?} {error}");
    assert!(!diagnostics.contains(secret));
    assert!(!diagnostics.contains("with-space"));
}

#[cfg(all(
    feature = "ai-gateway-http",
    not(target_family = "wasm"),
    any(target_os = "linux", target_os = "macos")
))]
mod composition {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use machine_god_core::{BoxFuture, CancellationToken, PermissionRequest};
    use machine_god_native::{
        AiGatewayByteStream, AiGatewayCredentialEnvironment, AiGatewayCredentialSource,
        AiGatewayTransport, AiGatewayTransportRequest, NativeReferenceHost,
        NativeReferenceHostBuildErrorKind, PermissionPromptDecision, PermissionPromptError,
        PermissionPrompter, QuestionPromptError, QuestionPromptOutcome, QuestionPromptRequest,
        QuestionPrompter,
    };

    use super::{
        AI_GATEWAY_DEFAULT_MODEL, ConfigOrigin, LoadedNativeConfig, OsString, TemporaryDirectory,
        load, never_deadline, production_gateway_target, valid_v1, valid_v2, valid_v3,
    };

    const OIDC_TOKEN: &str = "oidc-token_NEVER_REAL";
    const API_KEY: &str = "api-key_NEVER_REAL";

    #[derive(Clone, Default)]
    struct InertTransport {
        calls: Arc<AtomicUsize>,
    }

    impl AiGatewayTransport for InertTransport {
        fn stream(
            &self,
            _request: AiGatewayTransportRequest,
            _cancellation: CancellationToken,
        ) -> BoxFuture<'_, Result<AiGatewayByteStream, machine_god_core::ProviderError>> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            panic!("composition must not poll the network transport")
        }
    }

    #[derive(Clone, Default)]
    struct InertPrompter {
        calls: Arc<AtomicUsize>,
    }

    impl PermissionPrompter for InertPrompter {
        fn prompt(
            &self,
            _request: PermissionRequest,
        ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            panic!("composition must not poll the permission prompt")
        }
    }

    struct InertQuestionPrompter;

    impl QuestionPrompter for InertQuestionPrompter {
        fn prompt(
            &self,
            _request: QuestionPromptRequest,
        ) -> BoxFuture<'_, Result<QuestionPromptOutcome, QuestionPromptError>> {
            panic!("composition must not poll an ordinary question prompt")
        }
    }

    fn inert_question_prompter() -> Arc<dyn QuestionPrompter> {
        Arc::new(InertQuestionPrompter)
    }

    fn roots(temporary: &TemporaryDirectory) -> (PathBuf, PathBuf) {
        let workspace = temporary.path().join("workspace");
        let sessions = temporary.path().join("sessions");
        fs::create_dir(&workspace).unwrap();
        fs::create_dir(&sessions).unwrap();
        (workspace, sessions)
    }

    fn assert_inert(transport: &InertTransport, prompter: &InertPrompter, sessions: &Path) {
        assert_eq!(transport.calls.load(Ordering::Relaxed), 0);
        assert_eq!(prompter.calls.load(Ordering::Relaxed), 0);
        assert_eq!(fs::read_dir(sessions).unwrap().count(), 0);
    }

    fn load_fixture(
        temporary: &TemporaryDirectory,
        label: &str,
        contents: &str,
    ) -> LoadedNativeConfig {
        load(&temporary.path().join(label), contents.as_bytes())
    }

    #[test]
    fn schema_v3_production_composition_selects_oidc_then_empty_oidc_falls_back() {
        for (label, oidc, expected) in [
            (
                "oidc",
                Some(OsString::from(OIDC_TOKEN)),
                AiGatewayCredentialSource::VercelOidcToken,
            ),
            (
                "fallback",
                Some(OsString::new()),
                AiGatewayCredentialSource::AiGatewayApiKey,
            ),
        ] {
            let temporary = TemporaryDirectory::new(label);
            let loaded = load_fixture(
                &temporary,
                "config",
                &valid_v3("provider/configured-v3-model"),
            );
            let (workspace, sessions) = roots(&temporary);
            let prompter = InertPrompter::default();

            let host = NativeReferenceHost::compose_ai_gateway_http(
                loaded,
                AiGatewayCredentialEnvironment::new(oidc, Some(OsString::from(API_KEY))),
                &workspace,
                &sessions,
                Arc::new(prompter.clone()),
                inert_question_prompter(),
                never_deadline(),
            )
            .unwrap();

            assert_eq!(host.loaded_config().origin(), ConfigOrigin::File);
            assert_eq!(host.loaded_config().config().schema_version(), 3);
            assert_eq!(
                host.loaded_config().config().model(),
                "provider/configured-v3-model"
            );
            assert_eq!(host.credential_source(), Some(expected));
            assert_eq!(prompter.calls.load(Ordering::Relaxed), 0);
            assert_eq!(fs::read_dir(&sessions).unwrap().count(), 0);
        }
    }

    #[test]
    fn selected_invalid_oidc_fails_closed_without_api_key_fallback_or_other_effects() {
        let temporary = TemporaryDirectory::new("invalid-oidc");
        let loaded = load_fixture(&temporary, "config", &valid_v3(AI_GATEWAY_DEFAULT_MODEL));
        let (workspace, sessions) = roots(&temporary);
        let prompter = InertPrompter::default();
        let secret = "SELECTED_INVALID_SECRET\r\nINJECTION";

        let error = NativeReferenceHost::compose_ai_gateway_http(
            loaded,
            AiGatewayCredentialEnvironment::new(
                Some(OsString::from(secret)),
                Some(OsString::from(API_KEY)),
            ),
            &workspace,
            &sessions,
            Arc::new(prompter.clone()),
            inert_question_prompter(),
            never_deadline(),
        )
        .unwrap_err();

        assert_eq!(error.kind(), NativeReferenceHostBuildErrorKind::Credential);
        let diagnostics = format!("{error:?} {error}");
        assert!(!diagnostics.contains(secret));
        assert!(!diagnostics.contains("SELECTED_INVALID_SECRET"));
        assert_eq!(prompter.calls.load(Ordering::Relaxed), 0);
        assert_eq!(fs::read_dir(&sessions).unwrap().count(), 0);
    }

    #[test]
    fn legacy_v1_and_v2_compose_through_the_projected_environment_source() {
        for (label, contents, expected_version) in [
            ("v1", valid_v1().to_owned(), 1),
            ("v2", valid_v2("provider/legacy-model"), 2),
        ] {
            let temporary = TemporaryDirectory::new(label);
            let loaded = load_fixture(&temporary, "config", &contents);
            let (workspace, sessions) = roots(&temporary);
            let prompter = InertPrompter::default();

            let host = NativeReferenceHost::compose_ai_gateway_http(
                loaded,
                AiGatewayCredentialEnvironment::new(None, Some(OsString::from(API_KEY))),
                &workspace,
                &sessions,
                Arc::new(prompter.clone()),
                inert_question_prompter(),
                never_deadline(),
            )
            .unwrap();

            assert_eq!(
                host.loaded_config().config().schema_version(),
                expected_version
            );
            assert_eq!(
                host.credential_source(),
                Some(AiGatewayCredentialSource::AiGatewayApiKey)
            );
            assert_eq!(prompter.calls.load(Ordering::Relaxed), 0);
            assert_eq!(fs::read_dir(&sessions).unwrap().count(), 0);
        }
    }

    #[test]
    fn custom_transport_override_skips_credentials_network_prompt_and_sessions() {
        for (label, contents) in [
            ("v1", valid_v1().to_owned()),
            ("v2", valid_v2("provider/legacy-model")),
            ("v3", valid_v3("provider/configured-v3-model")),
        ] {
            let temporary = TemporaryDirectory::new(label);
            let loaded = load_fixture(&temporary, "config", &contents);
            let (workspace, sessions) = roots(&temporary);
            let transport = InertTransport::default();
            let prompter = InertPrompter::default();

            let host = NativeReferenceHost::compose_with_ai_gateway_transport(
                loaded,
                Arc::new(transport.clone()),
                production_gateway_target(),
                &workspace,
                &sessions,
                Arc::new(prompter.clone()),
                inert_question_prompter(),
                never_deadline(),
            )
            .unwrap();

            assert_eq!(host.credential_source(), None);
            assert_inert(&transport, &prompter, &sessions);
        }
    }
}
