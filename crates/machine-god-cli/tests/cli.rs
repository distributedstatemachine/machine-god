use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const IDENTITY: &str = "machine-god 0.1.0 (engine API 1)\n";
const HELP: &str = concat!(
    "machine-god 0.1.0\n",
    "Embeddable coding-agent engine\n",
    "\n",
    "Usage:\n",
    "  machine-god\n",
    "  machine-god help\n",
    "  machine-god status [--json]\n",
    "\n",
    "Commands:\n",
    "  help      Show this help\n",
    "  status    Show configuration and runtime information\n",
    "\n",
    "Options:\n",
    "  -h, --help       Show this help\n",
    "  -V, --version    Show version\n",
);
const INVALID_ARGUMENTS: &str = concat!(
    "machine-god: invalid arguments\n",
    "Usage: machine-god [help | --help | -h | --version | -V | status [--json]]\n",
);

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

fn assert_success(output: &Output, stdout: &str) {
    assert!(output.status.success(), "status: {:?}", output.status);
    assert_eq!(output.stdout, stdout.as_bytes());
    assert!(output.stderr.is_empty());
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
    let output = machine_god()
        .args(["status", "--json"])
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_STATE_HOME")
        .output()
        .unwrap();
    assert_success(
        &output,
        concat!(
            "{\"name\":\"machine-god\",\"version\":\"0.1.0\",",
            "\"engine_api_version\":1,\"permission_mode\":\"ask\",",
            "\"config_file\":{\"path\":null,\"state\":\"unavailable\"},",
            "\"state_directory\":{\"path\":null,\"state\":\"unavailable\"}}\n",
        ),
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
fn non_unicode_arguments_are_rejected_by_the_process_boundary() {
    use std::os::unix::ffi::OsStringExt;

    let output = machine_god()
        .arg(OsString::from_vec(vec![0xff]))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, INVALID_ARGUMENTS.as_bytes());
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
    symlink(target_file, config_path).unwrap();
    symlink(target_directory, state_path).unwrap();

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
}
