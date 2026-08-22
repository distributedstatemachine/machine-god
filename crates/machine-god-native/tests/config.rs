use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use machine_god_native::{
    AI_GATEWAY_DEFAULT_MODEL, AI_GATEWAY_MAX_MODEL_BYTES, CONFIG_SCHEMA_VERSION, ConfigOrigin,
    LoadedNativeConfig, MAX_CONFIG_BYTES, NativeConfigError, NativeConfigErrorKind,
    NativeEnvironment, NativeProviderKind, NativeTransportKind, PermissionMode, load_native_config,
};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new() -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("mgcfg-{}-{identifier}", std::process::id()));
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

fn environment(config_root: Option<&Path>, home: Option<&Path>) -> NativeEnvironment {
    NativeEnvironment::new(
        config_root.map(Path::as_os_str).map(OsString::from),
        None,
        home.map(Path::as_os_str).map(OsString::from),
    )
}

fn config_path(config_root: &Path) -> PathBuf {
    config_root.join("machine-god").join("config.json")
}

fn write_config(config_root: &Path, contents: &[u8]) -> PathBuf {
    let path = config_path(config_root);
    fs::create_dir_all(path.parent().expect("config file must have a parent")).unwrap();
    fs::write(&path, contents).unwrap();
    path
}

fn valid_v1_config_json() -> &'static str {
    r#"{"schema_version":1,"permission_mode":"ask"}"#
}

fn valid_v2_config_json(model: &str) -> String {
    format!(
        r#"{{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"{model}"}}"#
    )
}

fn raw_v2_config_json(
    schema_version: &str,
    permission_mode: &str,
    provider: &str,
    transport: &str,
    model: &str,
) -> String {
    format!(
        r#"{{"schema_version":{schema_version},"permission_mode":{permission_mode},"provider":{provider},"transport":{transport},"model":{model}}}"#
    )
}

fn load_error(environment: &NativeEnvironment) -> NativeConfigError {
    match load_native_config(environment) {
        Ok(_) => panic!("configuration unexpectedly loaded successfully"),
        Err(error) => error,
    }
}

fn expected_error_message(kind: NativeConfigErrorKind) -> &'static str {
    match kind {
        NativeConfigErrorKind::InvalidEnvironment => "native configuration environment is invalid",
        NativeConfigErrorKind::InvalidFileType => "native configuration path is not a regular file",
        NativeConfigErrorKind::Unreadable => "native configuration file is unreadable",
        NativeConfigErrorKind::TooLarge => "native configuration file is too large",
        NativeConfigErrorKind::InvalidFormat => "native configuration format is invalid",
        NativeConfigErrorKind::UnsupportedSchemaVersion => {
            "native configuration schema version is unsupported"
        }
    }
}

fn assert_error(error: NativeConfigError, kind: NativeConfigErrorKind) {
    assert_eq!(error.kind(), kind);
    assert_eq!(error.to_string(), expected_error_message(kind));
    assert_eq!(
        format!("{error:?}"),
        format!("NativeConfigError {{ kind: {kind:?} }}")
    );
}

fn assert_contents_error(config_root: &Path, contents: &[u8], kind: NativeConfigErrorKind) {
    write_config(config_root, contents);
    let error = load_error(&environment(Some(config_root), None));
    assert_error(error, kind);
}

fn assert_diagnostics_omit(error: NativeConfigError, forbidden: &[&str]) {
    let display = error.to_string();
    let debug = format!("{error:?}");

    for value in forbidden {
        assert!(
            !display.contains(value),
            "Display leaked forbidden text {value:?}: {display:?}"
        );
        assert!(
            !debug.contains(value),
            "Debug leaked forbidden text {value:?}: {debug:?}"
        );
    }
}

fn assert_loaded_config(
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
}

#[test]
fn public_schema_constants_and_composition_names_are_stable() {
    assert_eq!(CONFIG_SCHEMA_VERSION, 2);
    assert_eq!(AI_GATEWAY_DEFAULT_MODEL, "zai/glm-5.2");
    assert_eq!(AI_GATEWAY_MAX_MODEL_BYTES, 128);
    assert_eq!(PermissionMode::Ask.as_str(), "ask");
    assert_eq!(
        NativeProviderKind::VercelAiGateway.as_str(),
        "vercel_ai_gateway"
    );
    assert_eq!(
        NativeTransportKind::AiGatewayHttp.as_str(),
        "ai_gateway_http"
    );
}

#[test]
fn missing_file_uses_schema_v2_built_in_defaults_without_creating_paths() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("absent-xdg-root");
    let loaded = load_native_config(&environment(Some(&config_root), None)).unwrap();

    assert_loaded_config(
        &loaded,
        ConfigOrigin::BuiltInDefaults,
        2,
        AI_GATEWAY_DEFAULT_MODEL,
    );
    assert!(!config_root.exists());
}

#[test]
fn unavailable_home_uses_schema_v2_built_in_defaults() {
    let loaded = load_native_config(&NativeEnvironment::new(None, None, None)).unwrap();

    assert_loaded_config(
        &loaded,
        ConfigOrigin::BuiltInDefaults,
        2,
        AI_GATEWAY_DEFAULT_MODEL,
    );
}

#[test]
fn invalid_selected_xdg_root_fails_instead_of_falling_back_to_home() {
    let temporary = TemporaryDirectory::new();
    let home = temporary.path().join("home");
    write_config(
        &home.join(".config"),
        valid_v2_config_json(AI_GATEWAY_DEFAULT_MODEL).as_bytes(),
    );
    let selected_root = OsString::from("relative-secret-xdg-root");
    let environment =
        NativeEnvironment::new(Some(selected_root), None, Some(home.as_os_str().to_owned()));

    let error = load_error(&environment);
    assert_error(error, NativeConfigErrorKind::InvalidEnvironment);
    assert_diagnostics_omit(error, &["relative-secret-xdg-root"]);
}

#[test]
fn strict_schema_v1_is_loaded_with_fixed_composition_without_modifying_it() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let contents = valid_v1_config_json().as_bytes().to_vec();
    let path = write_config(&config_root, &contents);

    let loaded = load_native_config(&environment(Some(&config_root), None)).unwrap();

    assert_loaded_config(&loaded, ConfigOrigin::File, 1, AI_GATEWAY_DEFAULT_MODEL);
    assert_eq!(fs::read(path).unwrap(), contents);
}

#[test]
fn strict_schema_v2_custom_model_is_loaded_without_modifying_it() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let model = "custom/provider-model-2026";
    let contents = valid_v2_config_json(model).into_bytes();
    let path = write_config(&config_root, &contents);

    let loaded = load_native_config(&environment(Some(&config_root), None)).unwrap();

    assert_loaded_config(&loaded, ConfigOrigin::File, 2, model);
    assert_eq!(fs::read(path).unwrap(), contents);
}

#[test]
fn loaded_config_is_cloneable_with_owned_model_and_is_not_copy() {
    struct IfCopy;
    trait AmbiguousIfCopy<Marker> {
        fn assert_not_copy() {}
    }
    impl<T: ?Sized> AmbiguousIfCopy<()> for T {}
    impl<T: Copy> AmbiguousIfCopy<IfCopy> for T {}

    let _ = <LoadedNativeConfig as AmbiguousIfCopy<_>>::assert_not_copy;

    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let model = "owned-clone-model";
    write_config(&config_root, valid_v2_config_json(model).as_bytes());
    let loaded = load_native_config(&environment(Some(&config_root), None)).unwrap();
    let cloned = loaded.clone();

    drop(loaded);
    assert_loaded_config(&cloned, ConfigOrigin::File, 2, model);
}

#[test]
fn config_debug_representations_redact_the_model() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let model = "DEBUG_MODEL_SECRET_MARKER";
    write_config(&config_root, valid_v2_config_json(model).as_bytes());
    let loaded = load_native_config(&environment(Some(&config_root), None)).unwrap();

    let config_debug = format!("{:?}", loaded.config());
    let loaded_debug = format!("{loaded:?}");
    assert!(
        !config_debug.contains(model),
        "model leaked: {config_debug}"
    );
    assert!(
        !loaded_debug.contains(model),
        "model leaked: {loaded_debug}"
    );
}

#[test]
fn strict_schema_v1_rejects_each_schema_v2_composition_field() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let cases = [
        r#"{"schema_version":1,"permission_mode":"ask","provider":"vercel_ai_gateway"}"#,
        r#"{"schema_version":1,"permission_mode":"ask","transport":"ai_gateway_http"}"#,
        r#"{"schema_version":1,"permission_mode":"ask","model":"zai/glm-5.2"}"#,
    ];

    for contents in cases {
        assert_contents_error(
            &config_root,
            contents.as_bytes(),
            NativeConfigErrorKind::InvalidFormat,
        );
    }
}

#[test]
fn strict_schema_v1_rejects_unknown_duplicate_missing_and_wrong_types() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let cases = [
        r#"{"schema_version":1,"permission_mode":"ask","unexpected":true}"#,
        r#"{"schema_version":1,"schema_version":1,"permission_mode":"ask"}"#,
        r#"{"schema_version":1,"permission_mode":"ask","permission_mode":"ask"}"#,
        r#"{"permission_mode":"ask"}"#,
        r#"{"schema_version":1}"#,
        r"{}",
        r#"{"schema_version":"1","permission_mode":"ask"}"#,
        r#"{"schema_version":true,"permission_mode":"ask"}"#,
        r#"{"schema_version":1.0,"permission_mode":"ask"}"#,
        r#"{"schema_version":1,"permission_mode":true}"#,
        r#"{"schema_version":1,"permission_mode":1}"#,
        r#"{"schema_version":1,"permission_mode":["ask"]}"#,
        r#"{"schema_version":1,"permission_mode":{"ask":null}}"#,
    ];

    for contents in cases {
        assert_contents_error(
            &config_root,
            contents.as_bytes(),
            NativeConfigErrorKind::InvalidFormat,
        );
    }
}

#[test]
fn strict_schema_v2_rejects_each_missing_field_and_unknown_fields() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let cases = [
        r#"{"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2"}"#,
        r#"{"schema_version":2,"provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2"}"#,
        r#"{"schema_version":2,"permission_mode":"ask","transport":"ai_gateway_http","model":"zai/glm-5.2"}"#,
        r#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","model":"zai/glm-5.2"}"#,
        r#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http"}"#,
        r#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","unexpected":true}"#,
    ];

    for contents in cases {
        assert_contents_error(
            &config_root,
            contents.as_bytes(),
            NativeConfigErrorKind::InvalidFormat,
        );
    }
}

#[test]
fn strict_schema_v2_rejects_each_duplicate_field() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let cases = [
        r#"{"schema_version":2,"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2"}"#,
        r#"{"schema_version":2,"permission_mode":"ask","permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2"}"#,
        r#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2"}"#,
        r#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","transport":"ai_gateway_http","model":"zai/glm-5.2"}"#,
        r#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","model":"zai/glm-5.2"}"#,
    ];

    for contents in cases {
        assert_contents_error(
            &config_root,
            contents.as_bytes(),
            NativeConfigErrorKind::InvalidFormat,
        );
    }
}

#[test]
fn strict_schema_v2_rejects_wrong_field_types() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let cases = [
        raw_v2_config_json(
            r#""2""#,
            r#""ask""#,
            r#""vercel_ai_gateway""#,
            r#""ai_gateway_http""#,
            r#""zai/glm-5.2""#,
        ),
        raw_v2_config_json(
            "true",
            r#""ask""#,
            r#""vercel_ai_gateway""#,
            r#""ai_gateway_http""#,
            r#""zai/glm-5.2""#,
        ),
        raw_v2_config_json(
            "2.0",
            r#""ask""#,
            r#""vercel_ai_gateway""#,
            r#""ai_gateway_http""#,
            r#""zai/glm-5.2""#,
        ),
        raw_v2_config_json(
            "2",
            "true",
            r#""vercel_ai_gateway""#,
            r#""ai_gateway_http""#,
            r#""zai/glm-5.2""#,
        ),
        raw_v2_config_json(
            "2",
            r#"{"ask":null}"#,
            r#""vercel_ai_gateway""#,
            r#""ai_gateway_http""#,
            r#""zai/glm-5.2""#,
        ),
        raw_v2_config_json(
            "2",
            r#""ask""#,
            "1",
            r#""ai_gateway_http""#,
            r#""zai/glm-5.2""#,
        ),
        raw_v2_config_json(
            "2",
            r#""ask""#,
            r#"{"vercel_ai_gateway":null}"#,
            r#""ai_gateway_http""#,
            r#""zai/glm-5.2""#,
        ),
        raw_v2_config_json(
            "2",
            r#""ask""#,
            r#""vercel_ai_gateway""#,
            "[]",
            r#""zai/glm-5.2""#,
        ),
        raw_v2_config_json(
            "2",
            r#""ask""#,
            r#""vercel_ai_gateway""#,
            r#"{"ai_gateway_http":null}"#,
            r#""zai/glm-5.2""#,
        ),
        raw_v2_config_json(
            "2",
            r#""ask""#,
            r#""vercel_ai_gateway""#,
            r#""ai_gateway_http""#,
            "null",
        ),
    ];

    for contents in cases {
        assert_contents_error(
            &config_root,
            contents.as_bytes(),
            NativeConfigErrorKind::InvalidFormat,
        );
    }
}

#[test]
fn schema_v2_enum_values_require_exact_stable_spellings() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let cases = [
        raw_v2_config_json(
            "2",
            r#""Ask""#,
            r#""vercel_ai_gateway""#,
            r#""ai_gateway_http""#,
            r#""zai/glm-5.2""#,
        ),
        raw_v2_config_json(
            "2",
            r#""ask""#,
            r#""VercelAiGateway""#,
            r#""ai_gateway_http""#,
            r#""zai/glm-5.2""#,
        ),
        raw_v2_config_json(
            "2",
            r#""ask""#,
            r#""vercel-ai-gateway""#,
            r#""ai_gateway_http""#,
            r#""zai/glm-5.2""#,
        ),
        raw_v2_config_json(
            "2",
            r#""ask""#,
            r#""vercel_ai_gateway""#,
            r#""AiGatewayHttp""#,
            r#""zai/glm-5.2""#,
        ),
        raw_v2_config_json(
            "2",
            r#""ask""#,
            r#""vercel_ai_gateway""#,
            r#""ai-gateway-http""#,
            r#""zai/glm-5.2""#,
        ),
    ];

    for contents in cases {
        assert_contents_error(
            &config_root,
            contents.as_bytes(),
            NativeConfigErrorKind::InvalidFormat,
        );
    }
}

#[test]
fn model_accepts_the_length_endpoints_and_rejects_one_more_byte() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");

    let minimum = "!";
    write_config(&config_root, valid_v2_config_json(minimum).as_bytes());
    let loaded = load_native_config(&environment(Some(&config_root), None)).unwrap();
    assert_loaded_config(&loaded, ConfigOrigin::File, 2, minimum);

    let exact = "~".repeat(AI_GATEWAY_MAX_MODEL_BYTES);
    let contents = valid_v2_config_json(&exact).into_bytes();
    let path = write_config(&config_root, &contents);

    let loaded = load_native_config(&environment(Some(&config_root), None)).unwrap();
    assert_loaded_config(&loaded, ConfigOrigin::File, 2, &exact);
    assert_eq!(fs::read(&path).unwrap(), contents);

    let oversized = "m".repeat(AI_GATEWAY_MAX_MODEL_BYTES + 1);
    assert_contents_error(
        &config_root,
        valid_v2_config_json(&oversized).as_bytes(),
        NativeConfigErrorKind::InvalidFormat,
    );
}

#[test]
fn model_accepts_every_visible_ascii_byte() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let model: String = (b'!'..=b'~').map(char::from).collect();
    let encoded_model = serde_json::to_string(&model).unwrap();
    let contents = raw_v2_config_json(
        "2",
        r#""ask""#,
        r#""vercel_ai_gateway""#,
        r#""ai_gateway_http""#,
        &encoded_model,
    );
    write_config(&config_root, contents.as_bytes());

    let loaded = load_native_config(&environment(Some(&config_root), None)).unwrap();
    assert_loaded_config(&loaded, ConfigOrigin::File, 2, &model);
}

#[test]
fn model_rejects_empty_space_control_delete_and_non_ascii_values() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let cases = [
        r#"""#,
        r#""model with space""#,
        r#""control-\u001f""#,
        r#""delete-\u007f""#,
        r#""non-ascii-é""#,
    ];

    for model in cases {
        let contents = raw_v2_config_json(
            "2",
            r#""ask""#,
            r#""vercel_ai_gateway""#,
            r#""ai_gateway_http""#,
            model,
        );
        assert_contents_error(
            &config_root,
            contents.as_bytes(),
            NativeConfigErrorKind::InvalidFormat,
        );
    }
}

#[test]
fn unsupported_schema_version_has_a_distinct_error_kind() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let contents = r#"{"schema_version":3,"permission_mode":"ask"}"#;

    assert_contents_error(
        &config_root,
        contents.as_bytes(),
        NativeConfigErrorKind::UnsupportedSchemaVersion,
    );
}

#[test]
fn future_schema_is_classified_before_version_specific_fields() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let cases = [
        br#"{"schema_version":3,"permission_mode":"future","new_field":true}"#.as_slice(),
        br#"{"schema_version":18446744073709551616}"#.as_slice(),
        br#"{"schema_version":-1,"future_shape":[]}"#.as_slice(),
    ];

    for contents in cases {
        assert_contents_error(
            &config_root,
            contents,
            NativeConfigErrorKind::UnsupportedSchemaVersion,
        );
    }
}

#[test]
fn supported_schema_v2_validates_version_specific_fields() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let contents = raw_v2_config_json(
        "2",
        r#""ask""#,
        r#""future_provider""#,
        r#""ai_gateway_http""#,
        r#""zai/glm-5.2""#,
    );

    assert_contents_error(
        &config_root,
        contents.as_bytes(),
        NativeConfigErrorKind::InvalidFormat,
    );
}

#[test]
fn invalid_utf8_precedes_future_version_classification() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");

    assert_contents_error(
        &config_root,
        b"{\"schema_version\":3,\"future\":\"\xff\"}",
        NativeConfigErrorKind::InvalidFormat,
    );
}

#[test]
fn malformed_trailing_and_non_utf8_input_are_rejected() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let malformed = r#"{"schema_version":2"#.as_bytes().to_vec();
    let trailing = format!("{}\n{{}}", valid_v2_config_json(AI_GATEWAY_DEFAULT_MODEL));
    let mut non_utf8 = valid_v2_config_json(AI_GATEWAY_DEFAULT_MODEL).into_bytes();
    let ask_start = non_utf8
        .windows(3)
        .position(|window| window == b"ask")
        .expect("valid fixture contains ask");
    non_utf8[ask_start] = 0xff;

    for contents in [malformed, trailing.into_bytes(), non_utf8] {
        assert_contents_error(
            &config_root,
            &contents,
            NativeConfigErrorKind::InvalidFormat,
        );
    }
}

#[test]
fn exact_config_size_limit_is_accepted_and_one_additional_byte_is_rejected() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let maximum = MAX_CONFIG_BYTES;
    assert_eq!(maximum, 64 * 1024);

    let mut contents = valid_v2_config_json(AI_GATEWAY_DEFAULT_MODEL).into_bytes();
    assert!(contents.len() < maximum);
    contents.resize(maximum, b' ');
    let path = write_config(&config_root, &contents);
    let loaded = load_native_config(&environment(Some(&config_root), None)).unwrap();
    assert_loaded_config(&loaded, ConfigOrigin::File, 2, AI_GATEWAY_DEFAULT_MODEL);

    contents.push(b' ');
    fs::write(path, &contents).unwrap();
    let error = load_error(&environment(Some(&config_root), None));
    assert_error(error, NativeConfigErrorKind::TooLarge);
}

#[cfg(unix)]
#[test]
fn final_symlink_is_rejected_without_reading_its_target() {
    use std::os::unix::fs::symlink;

    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    let target = temporary.path().join("target.json");
    fs::write(&target, valid_v2_config_json(AI_GATEWAY_DEFAULT_MODEL)).unwrap();
    let path = config_path(&config_root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    symlink(target, path).unwrap();

    let error = load_error(&environment(Some(&config_root), None));
    assert_error(error, NativeConfigErrorKind::InvalidFileType);
}

#[test]
fn directory_at_config_path_is_rejected() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("xdg");
    fs::create_dir_all(config_path(&config_root)).unwrap();

    let error = load_error(&environment(Some(&config_root), None));
    assert_error(error, NativeConfigErrorKind::InvalidFileType);
}

#[cfg(unix)]
#[test]
fn unix_socket_at_config_path_is_rejected_without_blocking() {
    use std::os::unix::net::UnixListener;

    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("x");
    let path = config_path(&config_root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let _listener = UnixListener::bind(path).unwrap();

    let error = load_error(&environment(Some(&config_root), None));
    assert_error(error, NativeConfigErrorKind::InvalidFileType);
}

#[test]
fn inaccessible_metadata_is_unreadable_and_diagnostics_hide_os_details() {
    let temporary = TemporaryDirectory::new();
    let oversized_component = format!("RAW_PATH_SECRET_{}", "x".repeat(512));
    let config_root = temporary.path().join(oversized_component);

    let error = load_error(&environment(Some(&config_root), None));

    assert_error(error, NativeConfigErrorKind::Unreadable);
    assert_diagnostics_omit(
        error,
        &[
            "RAW_PATH_SECRET_",
            "File name too long",
            "file name too long",
            "filename too long",
            "os error",
            "ENAMETOOLONG",
        ],
    );
}

#[test]
fn invalid_format_diagnostics_hide_path_and_content() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("PATH_SECRET_MARKER");
    let contents = br#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"CONTENT_SECRET_MARKER""#;
    write_config(&config_root, contents);

    let error = load_error(&environment(Some(&config_root), None));

    assert_error(error, NativeConfigErrorKind::InvalidFormat);
    assert_diagnostics_omit(error, &["PATH_SECRET_MARKER", "CONTENT_SECRET_MARKER"]);
}

#[test]
fn loading_missing_config_writes_nothing_to_an_existing_root() {
    let temporary = TemporaryDirectory::new();
    let config_root = temporary.path().join("empty-xdg-root");
    fs::create_dir(&config_root).unwrap();

    let before = fs::read_dir(&config_root).unwrap().count();
    let loaded = load_native_config(&environment(Some(&config_root), None)).unwrap();
    let after = fs::read_dir(&config_root).unwrap().count();

    assert_loaded_config(
        &loaded,
        ConfigOrigin::BuiltInDefaults,
        2,
        AI_GATEWAY_DEFAULT_MODEL,
    );
    assert_eq!(before, 0);
    assert_eq!(after, 0);
    assert!(!config_path(&config_root).exists());
}
