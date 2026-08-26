use std::ffi::OsStr;
#[cfg(unix)]
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const IDENTITY: &str = "machine-god 0.1.0 (engine API 1)\n";
const PERMISSIONS: &str = concat!(
    "machine-god 0.1.0 (engine API 1)\n",
    "permission_mode: ask\n",
    "persistent_rules: unsupported\n",
    "runtime_grants: unavailable\n",
);
const PERMISSIONS_JSON: &str = concat!(
    "{\"name\":\"machine-god\",\"version\":\"0.1.0\",",
    "\"engine_api_version\":1,\"kind\":\"permissions\",",
    "\"permission_mode\":\"ask\",\"persistent_rules_supported\":false,",
    "\"runtime_grants_available\":false}\n",
);
const HELP: &str = concat!(
    "machine-god 0.1.0\n",
    "Embeddable coding-agent engine\n",
    "\n",
    "Usage:\n",
    "  machine-god\n",
    "  machine-god help\n",
    "  machine-god doctor [--json]\n",
    "  machine-god models [--json]\n",
    "  machine-god permissions [--json]\n",
    "  machine-god status [--json]\n",
    "\n",
    "Commands:\n",
    "  help         Show this help\n",
    "  doctor       Run local health and preflight checks\n",
    "  models       List available models\n",
    "  permissions  Show the permission mode and rules\n",
    "  status       Show configuration and runtime information\n",
    "\n",
    "Options:\n",
    "  -h, --help       Show this help\n",
    "  -V, --version    Show version\n",
);
const INVALID_ARGUMENTS: &str = concat!(
    "machine-god: invalid arguments\n",
    "Usage: machine-god [help | --help | -h | --version | -V | doctor [--json] | models [--json] | permissions [--json] | status [--json]]\n",
);
const CONFIG_FAILURE: &str = "machine-god: failed to load configuration\n";
const MAX_CONFIG_BYTES: usize = 64 * 1024;
const MAX_DOCTOR_OUTPUT_BYTES: usize = 4096;

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(test_name: &str) -> Self {
        let base = std::env::temp_dir().join("machine-god-cli-tests");
        fs::create_dir_all(&base).unwrap();
        loop {
            let id = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("{}-{test_name}-{id}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("failed to create {}: {error}", path.display()),
            }
        }
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("failed to remove {}: {error}", self.0.display());
        }
    }
}

fn machine_god() -> Command {
    Command::new(env!("CARGO_BIN_EXE_machine-god"))
}

fn run(arguments: &[&str]) -> Output {
    machine_god().args(arguments).output().unwrap()
}

fn run_with_roots(arguments: &[&str], config: &OsStr, state: &OsStr) -> Output {
    machine_god()
        .args(arguments)
        .env_remove("HOME")
        .env("XDG_CONFIG_HOME", config)
        .env("XDG_STATE_HOME", state)
        .output()
        .unwrap()
}

fn run_without_roots(arguments: &[&str]) -> Output {
    machine_god()
        .args(arguments)
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_STATE_HOME")
        .output()
        .unwrap()
}

fn doctor_command(config: &OsStr, state: &OsStr) -> Command {
    let mut command = machine_god();
    command
        .env_remove("HOME")
        .env("XDG_CONFIG_HOME", config)
        .env("XDG_STATE_HOME", state)
        .env_remove("VERCEL_OIDC_TOKEN")
        .env_remove("AI_GATEWAY_API_KEY");
    command
}

fn run_doctor(arguments: &[&str], config: &OsStr, state: &OsStr) -> Output {
    doctor_command(config, state)
        .args(arguments)
        .output()
        .unwrap()
}

fn run_models_with_invalid_credential(arguments: &[&str], config: &OsStr, state: &OsStr) -> Output {
    machine_god()
        .args(arguments)
        .env_remove("HOME")
        .env("XDG_CONFIG_HOME", config)
        .env("XDG_STATE_HOME", state)
        .env("VERCEL_OIDC_TOKEN", "CLI MODELS INVALID CREDENTIAL SECRET")
        .env_remove("AI_GATEWAY_API_KEY")
        .output()
        .unwrap()
}

fn config_path(config_root: &Path) -> PathBuf {
    config_root.join("machine-god/config.json")
}

fn write_config(config_root: &Path, contents: &[u8]) -> PathBuf {
    let path = config_path(config_root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, contents).unwrap();
    path
}

fn assert_success(output: &Output, stdout: &str) {
    assert!(output.status.success(), "status: {:?}", output.status);
    assert_eq!(output.stdout, stdout.as_bytes());
    assert!(output.stderr.is_empty());
}

fn assert_config_failure(output: &Output) {
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, CONFIG_FAILURE.as_bytes());
}

fn assert_models_unavailable(output: &Output, json: bool) {
    assert_eq!(output.status.code(), Some(1));
    if json {
        assert_eq!(
            output.stdout,
            concat!(
                "{\"kind\":\"models\",\"error\":",
                "\"could not list models: Unavailable\",\"code\":\"Unavailable\"}\n",
            )
            .as_bytes()
        );
        assert!(output.stderr.is_empty());
    } else {
        assert!(output.stdout.is_empty());
        assert_eq!(
            output.stderr,
            b"machine-god models: could not list models: Unavailable\n"
        );
    }
}

#[test]
fn identity_and_version_aliases_are_byte_stable() {
    for arguments in [&[][..], &["--version"][..], &["-V"][..]] {
        assert_success(&run(arguments), IDENTITY);
    }
}

#[test]
fn help_aliases_are_byte_stable() {
    for arguments in [&["help"][..], &["--help"][..], &["-h"][..]] {
        assert_success(&run(arguments), HELP);
    }
}

#[test]
fn malformed_arguments_have_one_diagnostic_and_exit_two() {
    for arguments in [
        &["unknown"][..],
        &["help", "extra"][..],
        &["--json", "status"][..],
        &["--json", "models"][..],
        &["--json", "permissions"][..],
        &["--json", "doctor"][..],
        &["doctor", "--json=true"][..],
        &["doctor", "extra"][..],
        &["doctor", "--json", "extra"][..],
        &["doctor", "--json", "--json"][..],
        &["models", "--json=true"][..],
        &["models", "extra"][..],
        &["models", "--json", "extra"][..],
        &["models", "--json", "--json"][..],
        &["permissions", "--json=true"][..],
        &["permissions", "extra"][..],
        &["permissions", "--json", "extra"][..],
        &["permissions", "--json", "--json"][..],
        &["status", "--json=true"][..],
        &["status", "--json", "extra"][..],
        &["status", "--json", "--json"][..],
    ] {
        let output = run(arguments);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, INVALID_ARGUMENTS.as_bytes());
    }
}

fn expected_doctor_output(
    config: (&str, &str),
    credential: (&str, &str),
    state: (&str, &str),
) -> (String, String) {
    let platform = if cfg!(any(target_os = "linux", target_os = "macos")) {
        ("ok", "native host platform is supported")
    } else {
        ("fail", "native host platform is unsupported")
    };
    let checks = [
        ("config", config.0, config.1),
        ("credential", credential.0, credential.1),
        ("state", state.0, state.1),
        ("platform", platform.0, platform.1),
    ];
    let count = |status: &str| {
        checks
            .iter()
            .filter(|(_, actual, _)| *actual == status)
            .count()
    };
    let (ok_count, warn_count, fail_count) = (count("ok"), count("warn"), count("fail"));

    let mut human = format!("[doctor] ok={ok_count} warn={warn_count} fail={fail_count}\n");
    let mut json = format!(
        "{{\"kind\":\"doctor\",\"ok_count\":{ok_count},\"warn_count\":{warn_count},\"fail_count\":{fail_count},\"checks\":["
    );
    for (index, (name, status, detail)) in checks.into_iter().enumerate() {
        writeln!(human, "[{status}] {name}: {detail}").expect("writing to a String cannot fail");
        if index != 0 {
            json.push(',');
        }
        write!(
            json,
            "{{\"name\":\"{name}\",\"status\":\"{status}\",\"detail\":\"{detail}\"}}"
        )
        .expect("writing to a String cannot fail");
    }
    json.push_str("]}\n");
    (human, json)
}

fn assert_doctor_success(output: &Output, expected: &str) {
    assert_success(output, expected);
    assert!(output.stdout.len() <= MAX_DOCTOR_OUTPUT_BYTES);
    assert_eq!(output.stdout.last(), Some(&b'\n'));
}

fn assert_output_omits(output: &Output, forbidden: &[&str]) {
    for bytes in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(bytes);
        for value in forbidden {
            assert!(
                !text.contains(value),
                "doctor output leaked forbidden value {value:?}: {text:?}"
            );
        }
    }
}

#[test]
fn doctor_missing_inputs_are_exact_counted_and_do_not_create_roots() {
    let temporary = TestDirectory::new("doctor-missing");
    let config_root = temporary.path().join("missing-config-PATH_MARKER");
    let state_root = temporary.path().join("missing-state-PATH_MARKER");
    let (human, json) = expected_doctor_output(
        (
            "warn",
            "configuration file is missing; using built-in defaults",
        ),
        ("fail", "no AI Gateway credential is configured"),
        ("warn", "state directory is not initialized"),
    );

    let human_output = run_doctor(&["doctor"], config_root.as_os_str(), state_root.as_os_str());
    assert_doctor_success(&human_output, &human);
    let json_output = run_doctor(
        &["doctor", "--json"],
        config_root.as_os_str(),
        state_root.as_os_str(),
    );
    assert_doctor_success(&json_output, &json);
    assert_eq!(json.matches("\"name\":").count(), 4);
    assert_eq!(json.matches("\"status\":\"ok\"").count(), 1);
    assert_eq!(json.matches("\"status\":\"warn\"").count(), 2);
    assert_eq!(json.matches("\"status\":\"fail\"").count(), 1);
    assert_output_omits(&human_output, &["PATH_MARKER"]);
    assert_output_omits(&json_output, &["PATH_MARKER"]);
    assert!(!config_root.exists());
    assert!(!state_root.exists());
}

#[test]
fn doctor_reads_each_strict_schema_without_rewrite_and_reports_oidc_precedence() {
    let schemas: [(&str, &[u8]); 3] = [
        ("v1", br#"{"schema_version":1,"permission_mode":"ask"}"#),
        (
            "v2",
            br#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"CLI_DOCTOR_V2_SECRET"}"#,
        ),
        (
            "v3",
            br#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"CLI_DOCTOR_V3_SECRET","credential_source":"environment"}"#,
        ),
    ];
    let (human, json) = expected_doctor_output(
        ("ok", "configuration file is valid"),
        ("ok", "VERCEL_OIDC_TOKEN is configured"),
        ("ok", "state directory is ready"),
    );

    for (schema, contents) in schemas {
        let temporary = TestDirectory::new(&format!("doctor-schema-{schema}"));
        let config_root = temporary.path().join("config");
        let state_root = temporary.path().join("state");
        let path = write_config(&config_root, contents);
        fs::create_dir_all(state_root.join("machine-god")).unwrap();

        let human_output = doctor_command(config_root.as_os_str(), state_root.as_os_str())
            .arg("doctor")
            .env("VERCEL_OIDC_TOKEN", "doctor-oidc_NEVER_REAL")
            .env("AI_GATEWAY_API_KEY", "doctor-api-key_NEVER_REAL")
            .output()
            .unwrap();
        assert_doctor_success(&human_output, &human);

        let json_output = doctor_command(config_root.as_os_str(), state_root.as_os_str())
            .args(["doctor", "--json"])
            .env("VERCEL_OIDC_TOKEN", "doctor-oidc_NEVER_REAL")
            .env("AI_GATEWAY_API_KEY", "doctor-api-key_NEVER_REAL")
            .output()
            .unwrap();
        assert_doctor_success(&json_output, &json);
        assert_output_omits(
            &json_output,
            &[
                "doctor-oidc_NEVER_REAL",
                "doctor-api-key_NEVER_REAL",
                "CLI_DOCTOR_V2_SECRET",
                "CLI_DOCTOR_V3_SECRET",
            ],
        );
        assert_eq!(fs::read(&path).unwrap(), contents);
    }
}

#[test]
fn doctor_api_key_and_invalid_selected_credential_are_redacted() {
    let temporary = TestDirectory::new("doctor-credentials");
    let config_root = temporary.path().join("missing-config");
    let state_root = temporary.path().join("missing-state");
    let common_config = (
        "warn",
        "configuration file is missing; using built-in defaults",
    );
    let common_state = ("warn", "state directory is not initialized");

    let (_, api_json) = expected_doctor_output(
        common_config,
        ("ok", "AI_GATEWAY_API_KEY is configured"),
        common_state,
    );
    let api_output = doctor_command(config_root.as_os_str(), state_root.as_os_str())
        .args(["doctor", "--json"])
        .env("AI_GATEWAY_API_KEY", "doctor-api-key_NEVER_REAL")
        .output()
        .unwrap();
    assert_doctor_success(&api_output, &api_json);
    assert_output_omits(&api_output, &["doctor-api-key_NEVER_REAL"]);

    let invalid_secret = "DOCTOR_INVALID_SELECTED_SECRET with space";
    let (_, invalid_json) = expected_doctor_output(
        common_config,
        ("fail", "AI Gateway bearer token is invalid"),
        common_state,
    );
    let invalid_output = doctor_command(config_root.as_os_str(), state_root.as_os_str())
        .args(["doctor", "--json"])
        .env("VERCEL_OIDC_TOKEN", invalid_secret)
        .env("AI_GATEWAY_API_KEY", "valid-lower-source_NEVER_REAL")
        .output()
        .unwrap();
    assert_doctor_success(&invalid_output, &invalid_json);
    assert_output_omits(
        &invalid_output,
        &[invalid_secret, "valid-lower-source_NEVER_REAL"],
    );

    let oversized_secret = "x".repeat(4097);
    let oversized_output = doctor_command(config_root.as_os_str(), state_root.as_os_str())
        .args(["doctor", "--json"])
        .env("VERCEL_OIDC_TOKEN", &oversized_secret)
        .output()
        .unwrap();
    assert_doctor_success(&oversized_output, &invalid_json);
    assert_output_omits(&oversized_output, &[&oversized_secret]);
    assert!(!config_root.exists());
    assert!(!state_root.exists());
}

#[test]
fn doctor_config_failures_are_report_data_and_never_reflect_inputs() {
    let mut oversized = vec![b'x'; MAX_CONFIG_BYTES + 1];
    let marker = b"CLI_DOCTOR_OVERSIZE_SECRET";
    oversized[..marker.len()].copy_from_slice(marker);
    let cases = [
        (
            "malformed",
            b"CLI_DOCTOR_MALFORMED_SECRET:not-json".to_vec(),
            "native configuration format is invalid",
            "CLI_DOCTOR_MALFORMED_SECRET",
        ),
        (
            "unsupported",
            br#"{"schema_version":7,"future":"CLI_DOCTOR_VERSION_SECRET"}"#.to_vec(),
            "native configuration schema version is unsupported",
            "CLI_DOCTOR_VERSION_SECRET",
        ),
        (
            "oversized",
            oversized,
            "native configuration file is too large",
            "CLI_DOCTOR_OVERSIZE_SECRET",
        ),
    ];

    for (case, contents, detail, secret) in cases {
        let temporary = TestDirectory::new(&format!("doctor-invalid-config-{case}"));
        let config_root = temporary.path().join("config-PATH_SECRET");
        let state_root = temporary.path().join("missing-state");
        let path = write_config(&config_root, &contents);
        let (_, expected) = expected_doctor_output(
            ("fail", detail),
            ("fail", "no AI Gateway credential is configured"),
            ("warn", "state directory is not initialized"),
        );

        let output = run_doctor(
            &["doctor", "--json"],
            config_root.as_os_str(),
            state_root.as_os_str(),
        );
        assert_doctor_success(&output, &expected);
        assert_output_omits(&output, &[secret, "PATH_SECRET"]);
        assert_eq!(fs::read(path).unwrap(), contents);
        assert!(!state_root.exists());
    }
}

#[test]
fn doctor_wrong_file_types_and_invalid_locations_are_exact_and_read_only() {
    let temporary = TestDirectory::new("doctor-wrong-types");
    let config_root = temporary.path().join("config");
    let state_root = temporary.path().join("state");
    let config_path = config_path(&config_root);
    let state_path = state_root.join("machine-god");
    fs::create_dir_all(&config_path).unwrap();
    fs::create_dir_all(&state_root).unwrap();
    fs::write(&state_path, b"CLI_DOCTOR_STATE_SECRET").unwrap();
    let (expected, _) = expected_doctor_output(
        ("fail", "native configuration path is not a regular file"),
        ("fail", "no AI Gateway credential is configured"),
        ("fail", "state path is not a directory"),
    );

    let output = run_doctor(&["doctor"], config_root.as_os_str(), state_root.as_os_str());
    assert_doctor_success(&output, &expected);
    assert_output_omits(&output, &["CLI_DOCTOR_STATE_SECRET"]);
    assert!(config_path.is_dir());
    assert_eq!(fs::read(state_path).unwrap(), b"CLI_DOCTOR_STATE_SECRET");

    let home = temporary.path().join("fallback-home");
    let (_, invalid_json) = expected_doctor_output(
        ("fail", "native configuration environment is invalid"),
        ("fail", "no AI Gateway credential is configured"),
        ("fail", "state directory environment is invalid"),
    );
    let invalid = machine_god()
        .args(["doctor", "--json"])
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", "relative-config-PATH_SECRET")
        .env("XDG_STATE_HOME", "relative-state-PATH_SECRET")
        .env_remove("VERCEL_OIDC_TOKEN")
        .env_remove("AI_GATEWAY_API_KEY")
        .output()
        .unwrap();
    assert_doctor_success(&invalid, &invalid_json);
    assert_output_omits(&invalid, &["PATH_SECRET"]);
    assert!(!home.exists());
}

#[test]
fn invalid_doctor_grammar_precedes_inspection_and_writes() {
    let temporary = TestDirectory::new("doctor-argument-precedence");
    let config_root = temporary.path().join("config");
    let state_root = temporary.path().join("missing-state");
    let contents = b"CLI_DOCTOR_ARGUMENT_SECRET:not-json";
    let path = write_config(&config_root, contents);

    for arguments in [
        &["doctor", "extra"][..],
        &["doctor", "--json=true"][..],
        &["doctor", "--json", "extra"][..],
        &["doctor", "--json", "--json"][..],
    ] {
        let output = run_doctor(arguments, config_root.as_os_str(), state_root.as_os_str());
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, INVALID_ARGUMENTS.as_bytes());
    }
    assert_eq!(fs::read(path).unwrap(), contents);
    assert!(!state_root.exists());
}

#[test]
fn invalid_models_arguments_precede_invalid_configuration() {
    let temporary = TestDirectory::new("models-argument-precedence");
    let config_root = temporary.path().join("config");
    let state_root = temporary.path().join("state");
    let contents = b"CLI_MODELS_ARGUMENT_PRECEDENCE_SECRET:not-json";
    let path = write_config(&config_root, contents);

    for arguments in [&["models", "extra"][..], &["models", "--json", "extra"][..]] {
        let output = run_with_roots(arguments, config_root.as_os_str(), state_root.as_os_str());
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, INVALID_ARGUMENTS.as_bytes());
    }

    assert_eq!(fs::read(path).unwrap(), contents);
    assert!(!state_root.exists());
}

#[test]
fn models_invalid_config_is_a_fixed_redacted_failure_without_writes() {
    let temporary = TestDirectory::new("models-invalid-config");
    let config_root = temporary.path().join("config");
    let state_root = temporary.path().join("state");
    let contents = b"CLI_MODELS_INVALID_CONFIG_SECRET:not-json";
    let path = write_config(&config_root, contents);

    for (arguments, json) in [(&["models"][..], false), (&["models", "--json"][..], true)] {
        let output = run_with_roots(arguments, config_root.as_os_str(), state_root.as_os_str());
        assert_models_unavailable(&output, json);
    }

    assert_eq!(fs::read(path).unwrap(), contents);
    assert!(!state_root.exists());
}

#[test]
fn models_invalid_credential_fails_before_network_without_creating_roots() {
    let temporary = TestDirectory::new("models-invalid-credential");
    let config_root = temporary.path().join("missing-config");
    let state_root = temporary.path().join("missing-state");

    let human = run_models_with_invalid_credential(
        &["models"],
        config_root.as_os_str(),
        state_root.as_os_str(),
    );
    assert_models_unavailable(&human, false);

    let json = run_models_with_invalid_credential(
        &["models", "--json"],
        config_root.as_os_str(),
        state_root.as_os_str(),
    );
    assert_models_unavailable(&json, true);

    assert!(!config_root.exists());
    assert!(!state_root.exists());
}

#[test]
fn models_reads_v1_v2_and_v3_without_rewrite_or_state_access() {
    let schemas: [(&str, &[u8]); 3] = [
        (
            "v1",
            br#"{"schema_version":1,"permission_mode":"ask"}"#,
        ),
        (
            "v2",
            br#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"CLI_MODELS_V2_MARKER"}"#,
        ),
        (
            "v3",
            br#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"CLI_MODELS_V3_MARKER","credential_source":"environment"}"#,
        ),
    ];

    for (schema, contents) in schemas {
        let temporary = TestDirectory::new(&format!("models-schema-{schema}"));
        let config_root = temporary.path().join("config");
        let state_root = temporary.path().join("missing-state");
        let path = write_config(&config_root, contents);

        for (arguments, json) in [(&["models"][..], false), (&["models", "--json"][..], true)] {
            let output = run_models_with_invalid_credential(
                arguments,
                config_root.as_os_str(),
                state_root.as_os_str(),
            );
            assert_models_unavailable(&output, json);
            assert_eq!(fs::read(&path).unwrap(), contents);
            assert!(!state_root.exists());
        }
        let entries = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_name(), "config.json");
    }
}

#[test]
fn permissions_missing_config_uses_exact_safe_defaults_without_writes() {
    let temporary = TestDirectory::new("permissions-missing");
    let config_root = temporary.path().join("missing-config");
    let state_root = temporary.path().join("missing-state");

    assert_success(
        &run_with_roots(
            &["permissions"],
            config_root.as_os_str(),
            state_root.as_os_str(),
        ),
        PERMISSIONS,
    );
    assert_success(
        &run_with_roots(
            &["permissions", "--json"],
            config_root.as_os_str(),
            state_root.as_os_str(),
        ),
        PERMISSIONS_JSON,
    );

    assert!(!config_root.exists());
    assert!(!state_root.exists());
}

#[test]
fn invalid_permissions_arguments_precede_invalid_configuration() {
    let temporary = TestDirectory::new("permissions-argument-precedence");
    let config_root = temporary.path().join("config");
    let state_root = temporary.path().join("state");
    let contents = b"CLI_ARGUMENT_PRECEDENCE_SECRET:not-json";
    let path = write_config(&config_root, contents);

    for arguments in [
        &["permissions", "extra"][..],
        &["permissions", "--json", "extra"][..],
    ] {
        let output = run_with_roots(arguments, config_root.as_os_str(), state_root.as_os_str());
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, INVALID_ARGUMENTS.as_bytes());
    }

    assert_eq!(fs::read(path).unwrap(), contents);
    assert!(!state_root.exists());
}

#[test]
fn permissions_reads_v1_v2_and_v3_without_rewrite_or_state_access() {
    let schemas: [(&str, &[u8]); 3] = [
        (
            "v1",
            br#"{"schema_version":1,"permission_mode":"ask"}"#,
        ),
        (
            "v2",
            br#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"CLI_V2_MODEL_MARKER"}"#,
        ),
        (
            "v3",
            br#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"CLI_V3_MODEL_MARKER","credential_source":"environment"}"#,
        ),
    ];

    for (schema, contents) in schemas {
        let temporary = TestDirectory::new(&format!("permissions-schema-{schema}"));
        let config_root = temporary.path().join("config");
        let state_root = temporary.path().join("missing-state");
        let path = write_config(&config_root, contents);

        assert_success(
            &run_with_roots(
                &["permissions"],
                config_root.as_os_str(),
                state_root.as_os_str(),
            ),
            PERMISSIONS,
        );
        assert_eq!(fs::read(&path).unwrap(), contents);
        assert!(!state_root.exists());

        assert_success(
            &run_with_roots(
                &["permissions", "--json"],
                config_root.as_os_str(),
                state_root.as_os_str(),
            ),
            PERMISSIONS_JSON,
        );
        assert_eq!(fs::read(&path).unwrap(), contents);
        assert!(!state_root.exists());
        let entries = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_name(), "config.json");
    }
}

#[test]
fn invalid_permission_configs_are_fixed_redacted_failures_without_writes() {
    let mut oversized = vec![b'x'; MAX_CONFIG_BYTES + 1];
    let oversize_marker = b"CLI_OVERSIZE_SECRET";
    oversized[..oversize_marker.len()].copy_from_slice(oversize_marker);
    let cases = [
        (
            "strict",
            br#"{"schema_version":3,"permission_mode":"deny","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"CLI_INVALID_CONFIG_SECRET","credential_source":"environment"}"#.to_vec(),
        ),
        (
            "malformed",
            br#"{"schema_version":3,"model":"CLI_MALFORMED_SECRET""#.to_vec(),
        ),
        (
            "non-utf8",
            b"{\"schema_version\":3,\"model\":\"CLI_NON_UTF8_SECRET\xff\"}".to_vec(),
        ),
        (
            "unsupported",
            br#"{"schema_version":4,"future_secret":"CLI_UNSUPPORTED_SECRET"}"#.to_vec(),
        ),
        ("oversized", oversized),
    ];

    for (case, contents) in cases {
        let temporary = TestDirectory::new(&format!("permissions-invalid-{case}"));
        let config_root = temporary.path().join("config");
        let state_root = temporary.path().join("state");
        let path = write_config(&config_root, &contents);

        for arguments in [&["permissions"][..], &["permissions", "--json"][..]] {
            let output = run_with_roots(arguments, config_root.as_os_str(), state_root.as_os_str());
            assert_config_failure(&output);
            assert_eq!(fs::read(&path).unwrap(), contents);
            assert!(!state_root.exists());
        }
    }
}

#[test]
fn permission_config_wrong_file_type_is_a_fixed_redacted_failure() {
    let temporary = TestDirectory::new("permissions-wrong-type");
    let config_root = temporary.path().join("config");
    let state_root = temporary.path().join("state");
    let path = config_path(&config_root);
    fs::create_dir_all(&path).unwrap();

    for arguments in [&["permissions"][..], &["permissions", "--json"][..]] {
        let output = run_with_roots(arguments, config_root.as_os_str(), state_root.as_os_str());
        assert_config_failure(&output);
    }

    assert!(path.is_dir());
    assert_eq!(fs::read_dir(path).unwrap().count(), 0);
    assert!(!state_root.exists());
}

#[test]
fn invalid_config_environment_is_a_fixed_redacted_failure_without_fallback() {
    let temporary = TestDirectory::new("permissions-invalid-environment");
    let state_root = temporary.path().join("state");

    for arguments in [&["permissions"][..], &["permissions", "--json"][..]] {
        let output = machine_god()
            .args(arguments)
            .env("HOME", temporary.path())
            .env("XDG_CONFIG_HOME", "CLI_RELATIVE_CONFIG_SECRET")
            .env("XDG_STATE_HOME", &state_root)
            .output()
            .unwrap();
        assert_config_failure(&output);
    }

    assert!(!state_root.exists());
    assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 0);
}

#[test]
fn missing_paths_are_reported_without_being_created() {
    let temporary = TestDirectory::new("missing");
    let config_root = temporary.path().join("missing-config");
    let state_root = temporary.path().join("missing-state");
    let config_path = config_root.join("machine-god/config.json");
    let state_path = state_root.join("machine-god");

    let output = run_with_roots(&["status"], config_root.as_os_str(), state_root.as_os_str());
    let expected = format!(
        concat!(
            "{IDENTITY}",
            "permission_mode: ask\n",
            "config_file: state=missing path={config_path:?}\n",
            "state_directory: state=missing path={state_path:?}\n",
        ),
        IDENTITY = IDENTITY,
        config_path = config_path.to_str().unwrap(),
        state_path = state_path.to_str().unwrap(),
    );
    assert_success(&output, &expected);
    assert!(!config_root.exists());
    assert!(!state_root.exists());
}

#[test]
fn json_status_is_compact_valid_and_has_fixed_shape() {
    let temporary = TestDirectory::new("json");
    let config_root = temporary.path().join("config");
    let state_root = temporary.path().join("state");
    let config_path = config_root.join("machine-god/config.json");
    let state_path = state_root.join("machine-god");
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    fs::create_dir_all(&state_path).unwrap();
    fs::write(&config_path, b"not parsed").unwrap();

    let output = run_with_roots(
        &["status", "--json"],
        config_root.as_os_str(),
        state_root.as_os_str(),
    );
    let expected = format!(
        concat!(
            "{{\"name\":\"machine-god\",\"version\":\"0.1.0\",",
            "\"engine_api_version\":1,\"permission_mode\":\"ask\",",
            "\"config_file\":{{\"path\":{config_path:?},\"state\":\"file\"}},",
            "\"state_directory\":{{\"path\":{state_path:?},\"state\":\"directory\"}}}}\n",
        ),
        config_path = config_path.to_str().unwrap(),
        state_path = state_path.to_str().unwrap(),
    );
    assert_success(&output, &expected);
}

#[test]
fn schema_v2_composition_does_not_change_status_bytes_or_rewrite_config() {
    let temporary = TestDirectory::new("schema-v2-status");
    let config_root = temporary.path().join("config");
    let state_root = temporary.path().join("state");
    let config_path = config_root.join("machine-god/config.json");
    let state_path = state_root.join("machine-god");
    let distinctive_model = "CLI_DISTINCTIVE_MODEL_MARKER";
    let contents = format!(
        concat!(
            "{{\"schema_version\":2,\"permission_mode\":\"ask\",",
            "\"provider\":\"vercel_ai_gateway\",",
            "\"transport\":\"ai_gateway_http\",",
            "\"model\":\"{distinctive_model}\"}}",
        ),
        distinctive_model = distinctive_model,
    )
    .into_bytes();
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    fs::create_dir_all(&state_path).unwrap();
    fs::write(&config_path, &contents).unwrap();

    let text = run_with_roots(&["status"], config_root.as_os_str(), state_root.as_os_str());
    let expected_text = format!(
        concat!(
            "{IDENTITY}",
            "permission_mode: ask\n",
            "config_file: state=file path={config_path:?}\n",
            "state_directory: state=directory path={state_path:?}\n",
        ),
        IDENTITY = IDENTITY,
        config_path = config_path.to_str().unwrap(),
        state_path = state_path.to_str().unwrap(),
    );
    assert_success(&text, &expected_text);

    let json = run_with_roots(
        &["status", "--json"],
        config_root.as_os_str(),
        state_root.as_os_str(),
    );
    let expected_json = format!(
        concat!(
            "{{\"name\":\"machine-god\",\"version\":\"0.1.0\",",
            "\"engine_api_version\":1,\"permission_mode\":\"ask\",",
            "\"config_file\":{{\"path\":{config_path:?},\"state\":\"file\"}},",
            "\"state_directory\":{{\"path\":{state_path:?},\"state\":\"directory\"}}}}\n",
        ),
        config_path = config_path.to_str().unwrap(),
        state_path = state_path.to_str().unwrap(),
    );
    assert_success(&json, &expected_json);

    for output in [&text.stdout, &json.stdout] {
        assert!(
            !output
                .windows(distinctive_model.len())
                .any(|window| window == distinctive_model.as_bytes())
        );
        assert!(
            !output
                .windows("vercel_ai_gateway".len())
                .any(|window| window == b"vercel_ai_gateway")
        );
        assert!(
            !output
                .windows("ai_gateway_http".len())
                .any(|window| window == b"ai_gateway_http")
        );
    }
    assert_eq!(fs::read(config_path).unwrap(), contents);
}

#[test]
fn unavailable_environment_uses_null_paths() {
    let output = run_without_roots(&["status", "--json"]);
    assert_success(
        &output,
        concat!(
            "{\"name\":\"machine-god\",\"version\":\"0.1.0\",",
            "\"engine_api_version\":1,\"permission_mode\":\"ask\",",
            "\"config_file\":{\"path\":null,\"state\":\"unavailable\"},",
            "\"state_directory\":{\"path\":null,\"state\":\"unavailable\"}}\n",
        ),
    );
    assert_success(&run_without_roots(&["permissions"]), PERMISSIONS);
    assert_success(
        &run_without_roots(&["permissions", "--json"]),
        PERMISSIONS_JSON,
    );
}

#[test]
fn relative_selected_roots_are_invalid_and_do_not_fall_back() {
    let temporary = TestDirectory::new("relative");
    let output = machine_god()
        .args(["status", "--json"])
        .env("HOME", temporary.path())
        .env("XDG_CONFIG_HOME", "relative-config")
        .env("XDG_STATE_HOME", "relative-state")
        .output()
        .unwrap();
    assert_success(
        &output,
        concat!(
            "{\"name\":\"machine-god\",\"version\":\"0.1.0\",",
            "\"engine_api_version\":1,\"permission_mode\":\"ask\",",
            "\"config_file\":{\"path\":null,\"state\":\"invalid_environment\"},",
            "\"state_directory\":{\"path\":null,\"state\":\"invalid_environment\"}}\n",
        ),
    );
}

#[cfg(unix)]
#[test]
fn status_escapes_path_control_characters() {
    let temporary = TestDirectory::new("escaping");
    let config_root = temporary
        .path()
        .join("config-\u{1b}[31m\nquoted-\"-\u{061c}-\u{202e}");
    let state_root = temporary.path().join("state-\\slash-\u{200f}-\u{2066}");

    for arguments in [&["status"][..], &["status", "--json"][..]] {
        let output = run_with_roots(arguments, config_root.as_os_str(), state_root.as_os_str());
        let stdout = String::from_utf8(output.stdout.clone()).unwrap();

        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        for raw_control in ['\u{1b}', '\u{061c}', '\u{200f}', '\u{202e}', '\u{2066}'] {
            assert!(!stdout.contains(raw_control));
        }
        assert!(stdout.contains("\\u001b[31m\\nquoted-\\\"-\\u061c-\\u202e"));
        assert!(stdout.contains("state-\\\\slash-\\u200f-\\u2066"));
    }
}

#[cfg(unix)]
#[test]
fn non_unicode_config_environment_is_a_fixed_redacted_failure() {
    use std::os::unix::ffi::OsStringExt;

    let temporary = TestDirectory::new("permissions-non-unicode-environment");
    let state_root = temporary.path().join("state");
    let config_root = OsString::from_vec(b"CLI_ENV_SECRET-\xff".to_vec());

    for arguments in [&["permissions"][..], &["permissions", "--json"][..]] {
        let output = machine_god()
            .args(arguments)
            .env("HOME", temporary.path())
            .env("XDG_CONFIG_HOME", &config_root)
            .env("XDG_STATE_HOME", &state_root)
            .output()
            .unwrap();
        assert_config_failure(&output);
    }

    assert!(!state_root.exists());
    assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn doctor_non_unicode_credential_and_roots_are_fixed_redacted_failures() {
    use std::os::unix::ffi::OsStringExt;

    let temporary = TestDirectory::new("doctor-non-unicode-environment");
    let config_root = temporary.path().join("missing-config");
    let state_root = temporary.path().join("missing-state");
    let credential_secret = OsString::from_vec(b"CLI_DOCTOR_CREDENTIAL_SECRET-\xff".to_vec());
    let (_, credential_json) = expected_doctor_output(
        (
            "warn",
            "configuration file is missing; using built-in defaults",
        ),
        ("fail", "AI Gateway credential environment is invalid"),
        ("warn", "state directory is not initialized"),
    );
    let credential_output = doctor_command(config_root.as_os_str(), state_root.as_os_str())
        .args(["doctor", "--json"])
        .env("VERCEL_OIDC_TOKEN", credential_secret)
        .env("AI_GATEWAY_API_KEY", "valid-lower-source_NEVER_REAL")
        .output()
        .unwrap();
    assert_doctor_success(&credential_output, &credential_json);
    assert_output_omits(
        &credential_output,
        &[
            "CLI_DOCTOR_CREDENTIAL_SECRET",
            "valid-lower-source_NEVER_REAL",
        ],
    );
    assert!(!config_root.exists());
    assert!(!state_root.exists());

    let mut invalid_root_bytes = temporary.path().as_os_str().as_encoded_bytes().to_vec();
    invalid_root_bytes.extend_from_slice(b"/CLI_DOCTOR_ROOT_SECRET-");
    invalid_root_bytes.push(0xff);
    let invalid_root = OsString::from_vec(invalid_root_bytes);
    let (_, roots_json) = expected_doctor_output(
        ("fail", "native configuration environment is invalid"),
        ("fail", "no AI Gateway credential is configured"),
        ("fail", "state directory environment is invalid"),
    );
    let roots_output = machine_god()
        .args(["doctor", "--json"])
        .env("HOME", temporary.path().join("fallback-home"))
        .env("XDG_CONFIG_HOME", &invalid_root)
        .env("XDG_STATE_HOME", &invalid_root)
        .env_remove("VERCEL_OIDC_TOKEN")
        .env_remove("AI_GATEWAY_API_KEY")
        .output()
        .unwrap();
    assert_doctor_success(&roots_output, &roots_json);
    assert_output_omits(&roots_output, &["CLI_DOCTOR_ROOT_SECRET"]);
    assert!(!temporary.path().join("fallback-home").exists());
}

#[cfg(unix)]
#[test]
fn unreadable_permission_config_is_redacted_when_modes_are_enforced() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = TestDirectory::new("permissions-unreadable");
    let config_root = temporary.path().join("config");
    let state_root = temporary.path().join("state");
    let contents = br#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"CLI_UNREADABLE_SECRET","credential_source":"environment"}"#;
    let path = write_config(&config_root, contents);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();

    if fs::File::open(&path).is_ok() {
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        return;
    }

    for arguments in [&["permissions"][..], &["permissions", "--json"][..]] {
        let output = run_with_roots(arguments, config_root.as_os_str(), state_root.as_os_str());
        assert_config_failure(&output);
    }

    assert!(!state_root.exists());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(fs::read(path).unwrap(), contents);
}

#[cfg(unix)]
#[test]
fn non_unicode_arguments_are_rejected_by_the_process_boundary() {
    use std::os::unix::ffi::OsStringExt;

    for arguments in [
        vec![OsString::from_vec(vec![0xff])],
        vec![OsString::from("doctor"), OsString::from_vec(vec![0xff])],
    ] {
        let output = machine_god().args(arguments).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, INVALID_ARGUMENTS.as_bytes());
    }
}

#[cfg(unix)]
#[test]
fn final_symlinks_are_reported_as_wrong_kinds() {
    use std::os::unix::fs::symlink;

    let temporary = TestDirectory::new("symlinks");
    let config_root = temporary.path().join("config");
    let state_root = temporary.path().join("state");
    let target_file = temporary.path().join("target.json");
    let target_directory = temporary.path().join("target-state");
    let config_path = config_root.join("machine-god/config.json");
    let state_path = state_root.join("machine-god");
    fs::write(&target_file, b"{}").unwrap();
    fs::create_dir(&target_directory).unwrap();
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    symlink(&target_file, config_path).unwrap();
    symlink(&target_directory, state_path).unwrap();

    let output = run_with_roots(
        &["status", "--json"],
        config_root.as_os_str(),
        state_root.as_os_str(),
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(stdout.contains("\"state\":\"not_file\""));
    assert!(stdout.contains("\"state\":\"not_directory\""));

    let permissions = run_with_roots(
        &["permissions"],
        config_root.as_os_str(),
        state_root.as_os_str(),
    );
    assert_config_failure(&permissions);
    assert_eq!(fs::read(target_file).unwrap(), b"{}");
    assert!(target_directory.is_dir());
}
