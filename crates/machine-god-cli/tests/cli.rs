use std::ffi::OsStr;
#[cfg(unix)]
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::future::Future;
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::pin::Pin;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::process::{Child, Stdio};
use std::process::{Command, Output};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::task::{Context, Poll, Wake, Waker};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::time::{Duration, Instant};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use machine_god_core::{
    Message, Role, SessionId, SessionIncarnationId, SessionRecord, SessionRevision, SessionStore,
    SessionStoreErrorKind,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use machine_god_native::{
    FileSessionStore, MAX_FILE_SESSION_BYTES, NativeEnvironment, NativeSessionListingErrorKind,
    list_native_sessions,
};

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
    "  machine-god session <id> [--json]\n",
    "  machine-god sessions [--json]\n",
    "  machine-god status [--json]\n",
    "\n",
    "Commands:\n",
    "  help         Show this help\n",
    "  doctor       Run local health and preflight checks\n",
    "  models       List available models\n",
    "  permissions  Show the permission mode and rules\n",
    "  session      Inspect a saved session\n",
    "  sessions     List saved sessions\n",
    "  status       Show configuration and runtime information\n",
    "\n",
    "Options:\n",
    "  -h, --help       Show this help\n",
    "  -V, --version    Show version\n",
);
const INVALID_ARGUMENTS: &str = concat!(
    "machine-god: invalid arguments\n",
    "Usage: machine-god [help | --help | -h | --version | -V | doctor [--json] | models [--json] | permissions [--json] | session <id> [--json] | sessions [--json] | status [--json]]\n",
);
const CONFIG_FAILURE: &str = "machine-god: failed to load configuration\n";
#[cfg(target_os = "linux")]
const OUTPUT_FAILURE: &str = "machine-god: failed to write output\n";
const MAX_CONFIG_BYTES: usize = 64 * 1024;
const MAX_DOCTOR_OUTPUT_BYTES: usize = 4096;
const MAX_SESSION_OUTPUT_BYTES: usize = 4096;

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

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct ScopedChild {
    child: Option<Child>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl ScopedChild {
    fn spawn(command: &mut Command) -> Self {
        Self {
            child: Some(command.spawn().unwrap()),
        }
    }

    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("child remains owned")
    }

    fn assert_running_for(&mut self, duration: Duration) {
        let deadline = Instant::now() + duration;
        loop {
            assert!(
                self.child_mut().try_wait().unwrap().is_none(),
                "child exited before the observation interval elapsed"
            );
            if Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn close_stdin_with(&mut self, bytes: &[u8]) {
        let mut stdin = self.child_mut().stdin.take().expect("child stdin is piped");
        std::io::Write::write_all(&mut stdin, bytes).unwrap();
    }

    fn wait_with_output(mut self, timeout: Duration) -> Output {
        let deadline = Instant::now() + timeout;
        loop {
            if self.child_mut().try_wait().unwrap().is_some() {
                return self
                    .child
                    .take()
                    .expect("child remains owned")
                    .wait_with_output()
                    .unwrap();
            }
            assert!(
                Instant::now() < deadline,
                "child did not exit within {timeout:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for ScopedChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take()
            && !matches!(child.try_wait(), Ok(Some(_)))
        {
            let _ = child.kill();
            let _ = child.wait();
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct NoopWake;

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ready<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    match Future::poll(Pin::as_mut(&mut future), &mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("session-store future unexpectedly remained pending"),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn private_directory(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::create_dir_all(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn save_session(state_base: &Path, value: &str) {
    let root = state_base.join("machine-god");
    private_directory(&root);
    let store = FileSessionStore::open(&root).unwrap();
    let record = SessionRecord::empty(
        SessionId::new(value).unwrap(),
        SessionIncarnationId::new(format!("incarnation-{value}")).unwrap(),
    );
    ready(store.save(record, None)).unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn session_artifacts(state_root: &Path) -> (PathBuf, PathBuf) {
    let entries = fs::read_dir(state_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    let data = entries
        .iter()
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .unwrap()
        .clone();
    let lock = data.with_extension("lock");
    (data, lock)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn save_session_summary(
    state_base: &Path,
    id: &str,
    incarnation_id: &str,
    revision: u64,
    next_turn_sequence: u64,
    message_count: usize,
    metadata_entry_count: usize,
) -> (PathBuf, PathBuf) {
    let root = state_base.join("machine-god");
    private_directory(&root);
    let store = FileSessionStore::open(&root).unwrap();
    let mut record = SessionRecord::empty(
        SessionId::new(id).unwrap(),
        SessionIncarnationId::new(incarnation_id).unwrap(),
    );
    record.next_turn_sequence = next_turn_sequence;
    record.messages = (0..message_count)
        .map(|index| Message::text(Role::User, format!("private-message-{index}")))
        .collect();
    for index in 0..metadata_entry_count {
        record
            .metadata
            .entry(format!("private-metadata-{index}"))
            .or_default();
    }

    let mut expected_revision = None;
    for expected in 1..=revision {
        let assigned = ready(store.save(record.clone(), expected_revision)).unwrap();
        assert_eq!(assigned, SessionRevision(expected));
        record.revision = assigned;
        expected_revision = Some(assigned);
    }
    session_artifacts(&root)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn seed_session_over_engine_defaults(state_base: &Path) -> (PathBuf, Vec<u8>) {
    let id = "over-engine-defaults";
    let incarnation_id = "incarnation-over-engine-defaults";
    let (record_path, _) = save_session_summary(state_base, id, incarnation_id, 1, 1, 0, 0);
    let limits = machine_god_core::EngineLimits::default();
    let message_count = limits.max_transcript_messages.get() + 1;
    let metadata_value_bytes = limits.max_session_metadata_bytes.get() + 1;
    assert_eq!(message_count, 4_097);
    assert!(metadata_value_bytes > 256 * 1_024);

    let mut record = String::with_capacity(metadata_value_bytes + message_count * 32);
    write!(
        record,
        concat!(
            "{{\"schema_version\":1,\"record\":{{",
            "\"id\":\"{id}\",\"incarnation_id\":\"{incarnation_id}\",",
            "\"revision\":11,\"next_turn_sequence\":8,\"messages\":[",
        ),
        id = id,
        incarnation_id = incarnation_id,
    )
    .expect("writing to a String cannot fail");
    for index in 0..message_count {
        if index != 0 {
            record.push(',');
        }
        record.push_str("{\"role\":\"user\",\"content\":[]}");
    }
    record.push_str("],\"metadata\":{\"oversized\":\"");
    record.extend(std::iter::repeat_n('m', metadata_value_bytes));
    record.push_str("\"}}}");

    let bytes = record.into_bytes();
    assert!(bytes.len() < MAX_FILE_SESSION_BYTES);
    fs::write(&record_path, &bytes).unwrap();
    (record_path, bytes)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn seed_manual_current_schema_record(
    state_base: &Path,
    id: &str,
    metadata: &str,
) -> (FileSessionStore, PathBuf, Vec<u8>) {
    let (record_path, _) =
        save_session_summary(state_base, id, &format!("incarnation-{id}"), 1, 1, 0, 0);
    let bytes = format!(
        concat!(
            "{{\"schema_version\":1,\"record\":{{",
            "\"id\":\"{id}\",\"incarnation_id\":\"incarnation-{id}\",",
            "\"revision\":9,\"next_turn_sequence\":6,",
            "\"messages\":[],\"metadata\":{metadata}}}}}",
        ),
        id = id,
        metadata = metadata,
    )
    .into_bytes();
    assert!(bytes.len() < MAX_FILE_SESSION_BYTES);
    fs::write(&record_path, &bytes).unwrap();

    let store = FileSessionStore::open(&state_base.join("machine-god")).unwrap();
    (store, record_path, bytes)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn assert_store_accepts_and_lists(
    store: &FileSessionStore,
    state_base: &Path,
    id: &str,
) -> SessionRecord {
    let session_id = SessionId::new(id).unwrap();
    let record = ready(store.load(session_id.clone())).unwrap().unwrap();
    let environment =
        NativeEnvironment::new(None, Some(state_base.as_os_str().to_os_string()), None);
    let listing = ready(list_native_sessions(environment)).unwrap();
    assert!(!listing.truncated());
    assert_eq!(listing.session_ids(), std::slice::from_ref(&session_id));
    record
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn assert_store_rejects_and_listing_agrees(store: &FileSessionStore, state_base: &Path, id: &str) {
    let error = ready(store.load(SessionId::new(id).unwrap())).unwrap_err();
    assert_eq!(error.kind, SessionStoreErrorKind::Corrupt);
    let environment =
        NativeEnvironment::new(None, Some(state_base.as_os_str().to_os_string()), None);
    let error = ready(list_native_sessions(environment)).unwrap_err();
    assert_eq!(error.kind(), NativeSessionListingErrorKind::Corrupt);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run_session_bounded(config: &OsStr, state: &OsStr, id: &str) -> Output {
    let mut command = session_command(config, state);
    command
        .args(["session", id, "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = ScopedChild::spawn(&mut command).wait_with_output(Duration::from_secs(10));
    assert!(output.stdout.len() <= MAX_SESSION_OUTPUT_BYTES);
    output
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn expected_session_json(id: &str, metadata_entry_count: usize) -> String {
    format!(
        concat!(
            "{{\"kind\":\"session\",\"id\":\"{id}\",",
            "\"incarnation_id\":\"incarnation-{id}\",",
            "\"revision\":9,\"next_turn_sequence\":6,",
            "\"message_count\":0,\"metadata_entry_count\":{metadata_entry_count}}}\n",
        ),
        id = id,
        metadata_entry_count = metadata_entry_count,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn assert_duplicate_metadata_case(id: &str, metadata: &str, final_secret: &str) {
    let temporary = TestDirectory::new(id);
    let state_base = temporary.path().join("state");
    let (store, record_path, record_before) =
        seed_manual_current_schema_record(&state_base, id, metadata);
    let record = assert_store_accepts_and_lists(&store, &state_base, id);
    assert_eq!(record.metadata.len(), 1);
    assert_eq!(
        record.metadata.get("same").and_then(|value| value.as_str()),
        Some(final_secret)
    );

    let output = run_session_bounded(
        OsStr::new("relative-config-must-not-be-read"),
        state_base.as_os_str(),
        id,
    );
    assert_success(&output, &expected_session_json(id, 1));
    assert_output_omits(&output, &["CLI_", final_secret]);
    assert_eq!(fs::read(record_path).unwrap(), record_before);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn wait_for_lock_holder_ready(child: &mut ScopedChild, ready_path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if fs::read(ready_path).is_ok_and(|contents| contents == b"ready\n") {
            assert!(
                child.child_mut().try_wait().unwrap().is_none(),
                "lock holder exited after signaling readiness"
            );
            return;
        }
        assert!(
            child.child_mut().try_wait().unwrap().is_none(),
            "lock holder exited before signaling readiness"
        );
        assert!(
            Instant::now() < deadline,
            "lock holder did not become ready within {timeout:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sessions_command(config: &OsStr, state: &OsStr) -> Command {
    let mut command = machine_god();
    command
        .env_remove("HOME")
        .env("XDG_CONFIG_HOME", config)
        .env("XDG_STATE_HOME", state)
        .env(
            "VERCEL_OIDC_TOKEN",
            "CLI_SESSIONS_IGNORED_INVALID CREDENTIAL",
        )
        .env(
            "AI_GATEWAY_API_KEY",
            "CLI_SESSIONS_IGNORED_LOWER_CREDENTIAL",
        );
    command
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn session_command(config: &OsStr, state: &OsStr) -> Command {
    let mut command = machine_god();
    command
        .env_remove("HOME")
        .env("XDG_CONFIG_HOME", config)
        .env("XDG_STATE_HOME", state)
        .env(
            "VERCEL_OIDC_TOKEN",
            "CLI_SESSION_IGNORED_INVALID CREDENTIAL",
        )
        .env("AI_GATEWAY_API_KEY", "CLI_SESSION_IGNORED_LOWER_CREDENTIAL");
    command
}

fn assert_sessions_error(output: &Output, json: bool, category: &str) {
    assert_eq!(output.status.code(), Some(1));
    let message = format!("could not list sessions: {category}");
    if json {
        assert_eq!(
            output.stdout,
            format!("{{\"kind\":\"sessions\",\"error\":\"{message}\",\"code\":\"{category}\"}}\n")
                .as_bytes()
        );
        assert!(output.stderr.is_empty());
    } else {
        assert!(output.stdout.is_empty());
        assert_eq!(
            output.stderr,
            format!("machine-god sessions: {message}\n").as_bytes()
        );
    }
}

fn assert_session_error(output: &Output, json: bool, category: &str) {
    assert_eq!(output.status.code(), Some(1));
    let message = format!("could not inspect session: {category}");
    if json {
        assert_eq!(
            output.stdout,
            format!("{{\"kind\":\"session\",\"error\":\"{message}\",\"code\":\"{category}\"}}\n")
                .as_bytes()
        );
        assert!(output.stderr.is_empty());
        assert!(output.stdout.len() <= MAX_SESSION_OUTPUT_BYTES);
    } else {
        assert!(output.stdout.is_empty());
        assert_eq!(
            output.stderr,
            format!("machine-god session: {message}\n").as_bytes()
        );
    }
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
        &["session"][..],
        &["session", "last"][..],
        &["session", "--id"][..],
        &["session", "--id", "alpha"][..],
        &["session", "--json"][..],
        &["session", "--json", "alpha"][..],
        &["session", "alpha", "--json=true"][..],
        &["session", "alpha", "extra"][..],
        &["session", "alpha", "--json", "extra"][..],
        &["session", "alpha", "--json", "--json"][..],
        &["sessions", "--json=true"][..],
        &["sessions", "extra"][..],
        &["sessions", "--json", "extra"][..],
        &["sessions", "--json", "--json"][..],
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn session_exact_outputs_use_xdg_state_and_create_only_the_private_lock() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = TestDirectory::new("session-exact");
    let config_root = temporary.path().join("config-CLI_SESSION_CONFIG_PATH");
    let config_contents = b"CLI_SESSION_INVALID_CONFIG_SECRET:not-json";
    let config_path = write_config(&config_root, config_contents);
    let xdg_state = temporary.path().join("state-CLI_SESSION_STATE_PATH");
    let home = temporary.path().join("home");
    let (record_path, lock_path) =
        save_session_summary(&xdg_state, "alpha", "incarnation-alpha", 7, 4, 3, 2);
    let (_, home_lock) = save_session_summary(
        &home.join(".local/state"),
        "alpha",
        "home-must-not-be-read",
        1,
        1,
        0,
        0,
    );
    fs::remove_file(&lock_path).unwrap();
    fs::remove_file(&home_lock).unwrap();
    let record_before = fs::read(&record_path).unwrap();

    let mut human_command = machine_god();
    human_command
        .args(["session", "alpha"])
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &config_root)
        .env("XDG_STATE_HOME", &xdg_state)
        .env(
            "VERCEL_OIDC_TOKEN",
            "CLI_SESSION_IGNORED_INVALID CREDENTIAL",
        )
        .env("AI_GATEWAY_API_KEY", "CLI_SESSION_IGNORED_LOWER_CREDENTIAL");
    let human = human_command.output().unwrap();
    assert_success(
        &human,
        concat!(
            "[session] alpha\n",
            " - incarnation_id: incarnation-alpha\n",
            " - revision: 7\n",
            " - next_turn_sequence: 4\n",
            " - message_count: 3\n",
            " - metadata_entry_count: 2\n",
        ),
    );
    assert!(human.stdout.len() <= MAX_SESSION_OUTPUT_BYTES);

    let mut json_command = machine_god();
    json_command
        .args(["session", "alpha", "--json"])
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &config_root)
        .env("XDG_STATE_HOME", &xdg_state)
        .env(
            "VERCEL_OIDC_TOKEN",
            "CLI_SESSION_IGNORED_INVALID CREDENTIAL",
        )
        .env("AI_GATEWAY_API_KEY", "CLI_SESSION_IGNORED_LOWER_CREDENTIAL");
    let json = json_command.output().unwrap();
    assert_success(
        &json,
        concat!(
            "{\"kind\":\"session\",\"id\":\"alpha\",",
            "\"incarnation_id\":\"incarnation-alpha\",\"revision\":7,",
            "\"next_turn_sequence\":4,\"message_count\":3,",
            "\"metadata_entry_count\":2}\n",
        ),
    );
    assert!(json.stdout.len() <= MAX_SESSION_OUTPUT_BYTES);

    for output in [&human, &json] {
        assert_output_omits(
            output,
            &[
                "CLI_SESSION_CONFIG_PATH",
                "CLI_SESSION_STATE_PATH",
                "CLI_SESSION_INVALID_CONFIG_SECRET",
                "CLI_SESSION_IGNORED_INVALID CREDENTIAL",
                "CLI_SESSION_IGNORED_LOWER_CREDENTIAL",
                "private-message",
                "private-metadata",
                "home-must-not-be-read",
            ],
        );
    }
    assert_eq!(fs::read(&record_path).unwrap(), record_before);
    assert_eq!(
        fs::metadata(&lock_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(!home_lock.exists());
    assert_eq!(fs::read(config_path).unwrap(), config_contents);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn session_waits_for_the_store_lock_and_succeeds_after_release() {
    const LOCK_HOLDER: &str = concat!(
        "import fcntl, os, sys\n",
        "lock = open(sys.argv[1], 'r+b', buffering=0)\n",
        "fcntl.flock(lock.fileno(), fcntl.LOCK_EX)\n",
        "with open(sys.argv[2], 'x', encoding='utf-8') as signal:\n",
        "    signal.write('ready\\n')\n",
        "    signal.flush()\n",
        "    os.fsync(signal.fileno())\n",
        "print('ready', flush=True)\n",
        "sys.stdin.buffer.read(1)\n",
        "fcntl.flock(lock.fileno(), fcntl.LOCK_UN)\n",
    );

    let temporary = TestDirectory::new("session-lock-wait");
    let config_root = temporary.path().join("missing-config");
    let state_base = temporary.path().join("state");
    let (record_path, lock_path) =
        save_session_summary(&state_base, "alpha", "incarnation-alpha", 1, 1, 0, 0);
    let record_before = fs::read(&record_path).unwrap();
    let ready_path = temporary.path().join("lock-holder-ready");

    let mut holder_command = Command::new("python3");
    holder_command
        .args(["-c", LOCK_HOLDER])
        .arg(&lock_path)
        .arg(&ready_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut holder = ScopedChild::spawn(&mut holder_command);
    wait_for_lock_holder_ready(&mut holder, &ready_path, Duration::from_secs(10));

    let mut inspect_command = session_command(config_root.as_os_str(), state_base.as_os_str());
    inspect_command
        .args(["session", "alpha"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut inspection = ScopedChild::spawn(&mut inspect_command);
    inspection.assert_running_for(Duration::from_millis(500));

    holder.close_stdin_with(b"release\n");
    let holder_output = holder.wait_with_output(Duration::from_secs(10));
    assert!(holder_output.status.success());
    assert_eq!(holder_output.stdout, b"ready\n");
    assert!(holder_output.stderr.is_empty());

    let output = inspection.wait_with_output(Duration::from_secs(10));
    assert_success(
        &output,
        concat!(
            "[session] alpha\n",
            " - incarnation_id: incarnation-alpha\n",
            " - revision: 1\n",
            " - next_turn_sequence: 1\n",
            " - message_count: 0\n",
            " - metadata_entry_count: 0\n",
        ),
    );
    assert_eq!(fs::read(record_path).unwrap(), record_before);
    assert!(!config_root.exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn session_inspects_store_valid_records_over_engine_defaults_without_rewrite() {
    let temporary = TestDirectory::new("session-over-engine-defaults");
    let config_root = temporary.path().join("missing-config");
    let state_base = temporary.path().join("state");
    let (record_path, record_before) = seed_session_over_engine_defaults(&state_base);

    let human = session_command(config_root.as_os_str(), state_base.as_os_str())
        .args(["session", "over-engine-defaults"])
        .output()
        .unwrap();
    assert_success(
        &human,
        concat!(
            "[session] over-engine-defaults\n",
            " - incarnation_id: incarnation-over-engine-defaults\n",
            " - revision: 11\n",
            " - next_turn_sequence: 8\n",
            " - message_count: 4097\n",
            " - metadata_entry_count: 1\n",
        ),
    );
    assert!(human.stdout.len() <= MAX_SESSION_OUTPUT_BYTES);
    assert_eq!(fs::read(&record_path).unwrap(), record_before);

    let json = session_command(config_root.as_os_str(), state_base.as_os_str())
        .args(["session", "over-engine-defaults", "--json"])
        .output()
        .unwrap();
    assert_success(
        &json,
        concat!(
            "{\"kind\":\"session\",\"id\":\"over-engine-defaults\",",
            "\"incarnation_id\":\"incarnation-over-engine-defaults\",",
            "\"revision\":11,\"next_turn_sequence\":8,",
            "\"message_count\":4097,\"metadata_entry_count\":1}\n",
        ),
    );
    assert!(json.stdout.len() <= MAX_SESSION_OUTPUT_BYTES);
    assert_eq!(fs::read(record_path).unwrap(), record_before);
    assert!(!config_root.exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn session_store_equivalence_rejects_an_out_of_range_number() {
    let temporary = TestDirectory::new("session-number-rejected");
    let state_base = temporary.path().join("state");
    let (store, record_path, record_before) = seed_manual_current_schema_record(
        &state_base,
        "number-rejected",
        r#"{"value":1.7976931348623158e308}"#,
    );
    assert_store_rejects_and_listing_agrees(&store, &state_base, "number-rejected");
    let output = run_session_bounded(
        OsStr::new("relative-config-must-not-be-read"),
        state_base.as_os_str(),
        "number-rejected",
    );
    assert_session_error(&output, true, "Corrupt");
    assert_output_omits(&output, &["1.7976931348623158e308"]);
    assert_eq!(fs::read(record_path).unwrap(), record_before);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn session_store_equivalence_accepts_the_maximum_finite_number() {
    let temporary = TestDirectory::new("session-number-accepted");
    let state_base = temporary.path().join("state");
    let (store, record_path, record_before) = seed_manual_current_schema_record(
        &state_base,
        "number-accepted",
        r#"{"value":1.79769313486231581e308}"#,
    );
    let record = assert_store_accepts_and_lists(&store, &state_base, "number-accepted");
    assert_eq!(record.metadata.len(), 1);
    let output = run_session_bounded(
        OsStr::new("relative-config-must-not-be-read"),
        state_base.as_os_str(),
        "number-accepted",
    );
    assert_success(&output, &expected_session_json("number-accepted", 1));
    assert_output_omits(&output, &["1.79769313486231581e308"]);
    assert_eq!(fs::read(record_path).unwrap(), record_before);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn session_store_equivalence_uses_last_literal_metadata_value() {
    assert_duplicate_metadata_case(
        "duplicate-literal",
        r#"{"same":"CLI_DUPLICATE_OLD_SECRET","same":"CLI_DUPLICATE_FINAL_SECRET"}"#,
        "CLI_DUPLICATE_FINAL_SECRET",
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn session_store_equivalence_uses_last_escaped_equivalent_metadata_value() {
    assert_duplicate_metadata_case(
        "duplicate-escaped",
        r#"{"same":"CLI_ESCAPED_OLD_SECRET","s\u0061me":"CLI_ESCAPED_FINAL_SECRET"}"#,
        "CLI_ESCAPED_FINAL_SECRET",
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn session_store_equivalence_ignores_shadowed_json_nodes() {
    let over_nodes = std::iter::repeat_n("null", 65_536)
        .collect::<Vec<_>>()
        .join(",");
    let temporary = TestDirectory::new("session-shadowed-over-nodes");
    let state_base = temporary.path().join("state");
    let metadata = format!(
        "{{\"payload\":{{\"same\":[{over_nodes}],\"s\\u0061me\":\"CLI_SHADOWED_FINAL_SECRET\"}}}}"
    );
    let (store, record_path, record_before) =
        seed_manual_current_schema_record(&state_base, "shadowed-over-nodes", &metadata);
    let record = assert_store_accepts_and_lists(&store, &state_base, "shadowed-over-nodes");
    assert_eq!(record.metadata.len(), 1);
    assert_eq!(
        record
            .metadata
            .get("payload")
            .and_then(|value| value.get("same"))
            .and_then(|value| value.as_str()),
        Some("CLI_SHADOWED_FINAL_SECRET")
    );
    let output = run_session_bounded(
        OsStr::new("relative-config-must-not-be-read"),
        state_base.as_os_str(),
        "shadowed-over-nodes",
    );
    assert_success(&output, &expected_session_json("shadowed-over-nodes", 1));
    assert_output_omits(&output, &["CLI_SHADOWED_FINAL_SECRET"]);
    assert_eq!(fs::read(record_path).unwrap(), record_before);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn session_store_equivalence_rejects_final_json_node_overflow() {
    let over_nodes = std::iter::repeat_n("null", 65_536)
        .collect::<Vec<_>>()
        .join(",");
    let temporary = TestDirectory::new("session-final-over-nodes");
    let state_base = temporary.path().join("state");
    let metadata = format!(
        "{{\"payload\":{{\"same\":\"CLI_SHADOWED_OLD_SECRET\",\"s\\u0061me\":[{over_nodes}]}}}}"
    );
    let (store, record_path, record_before) =
        seed_manual_current_schema_record(&state_base, "final-over-nodes", &metadata);
    assert_store_rejects_and_listing_agrees(&store, &state_base, "final-over-nodes");
    let output = run_session_bounded(
        OsStr::new("relative-config-must-not-be-read"),
        state_base.as_os_str(),
        "final-over-nodes",
    );
    assert_session_error(&output, true, "Corrupt");
    assert_output_omits(&output, &["CLI_SHADOWED_OLD_SECRET"]);
    assert_eq!(fs::read(record_path).unwrap(), record_before);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn session_missing_root_and_record_are_exact_not_found_without_creation() {
    let temporary = TestDirectory::new("session-not-found");
    let config_root = temporary.path().join("missing-config");
    let missing_state = temporary.path().join("missing-state");

    for (arguments, json) in [
        (&["session", "missing"][..], false),
        (&["session", "missing", "--json"][..], true),
    ] {
        let output = session_command(config_root.as_os_str(), missing_state.as_os_str())
            .args(arguments)
            .output()
            .unwrap();
        assert_session_error(&output, json, "NotFound");
    }
    assert!(!missing_state.exists());
    assert!(!config_root.exists());

    let existing_state = temporary.path().join("existing-state");
    let existing_root = existing_state.join("machine-god");
    private_directory(&existing_root);
    let output = session_command(config_root.as_os_str(), existing_state.as_os_str())
        .args(["session", "missing", "--json"])
        .output()
        .unwrap();
    assert_session_error(&output, true, "NotFound");
    assert_eq!(fs::read_dir(existing_root).unwrap().count(), 0);
    assert!(!config_root.exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn session_home_fallback_is_exact_and_bounded_for_maximum_identifiers() {
    let temporary = TestDirectory::new("session-home-fallback");
    let home = temporary.path().join("home");
    let id = "x".repeat(128);
    let incarnation_id = "i".repeat(128);
    save_session_summary(&home.join(".local/state"), &id, &incarnation_id, 1, 1, 0, 0);

    let output = machine_god()
        .args(["session", &id, "--json"])
        .env("HOME", &home)
        .env_remove("XDG_STATE_HOME")
        .env("XDG_CONFIG_HOME", "relative-config-must-not-be-read")
        .env("VERCEL_OIDC_TOKEN", "invalid credential must not be read")
        .output()
        .unwrap();
    let expected = format!(
        "{{\"kind\":\"session\",\"id\":\"{id}\",\"incarnation_id\":\"{incarnation_id}\",\"revision\":1,\"next_turn_sequence\":1,\"message_count\":0,\"metadata_entry_count\":0}}\n"
    );
    assert_success(&output, &expected);
    assert!(output.stdout.len() <= MAX_SESSION_OUTPUT_BYTES);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn invalid_session_grammar_precedes_corrupt_state_and_all_writes() {
    let temporary = TestDirectory::new("session-argument-precedence");
    let config_root = temporary.path().join("missing-config");
    let state_base = temporary.path().join("state");
    let (record_path, lock_path) =
        save_session_summary(&state_base, "alpha", "incarnation-alpha", 1, 1, 0, 0);
    let contents = b"CLI_SESSION_ARGUMENT_PRECEDENCE_SECRET:not-json";
    fs::write(&record_path, contents).unwrap();
    fs::remove_file(&lock_path).unwrap();

    for arguments in [
        &["session"][..],
        &["session", "last"][..],
        &["session", "--id"][..],
        &["session", "--id", "alpha"][..],
        &["session", "--json"][..],
        &["session", "--json", "alpha"][..],
        &["session", "bad/id"][..],
        &["session", "alpha", "--json=true"][..],
        &["session", "alpha", "extra"][..],
        &["session", "alpha", "--json", "extra"][..],
        &["session", "alpha", "--json", "--json"][..],
    ] {
        let output = session_command(config_root.as_os_str(), state_base.as_os_str())
            .args(arguments)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, INVALID_ARGUMENTS.as_bytes());
    }
    let too_long = "x".repeat(129);
    let output = session_command(config_root.as_os_str(), state_base.as_os_str())
        .args(["session", &too_long])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, INVALID_ARGUMENTS.as_bytes());

    assert_eq!(fs::read(record_path).unwrap(), contents);
    assert!(!lock_path.exists());
    assert!(!config_root.exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn session_corrupt_and_wrong_id_records_are_fixed_and_redacted() {
    let malformed = TestDirectory::new("session-malformed");
    let malformed_state = malformed
        .path()
        .join("state-CLI_SESSION_MALFORMED_STATE_SECRET");
    let (malformed_record, _) =
        save_session_summary(&malformed_state, "alpha", "incarnation-alpha", 1, 1, 0, 0);
    fs::write(
        malformed_record,
        b"CLI_SESSION_MALFORMED_RECORD_SECRET:not-json",
    )
    .unwrap();

    let wrong_id = TestDirectory::new("session-wrong-id");
    let wrong_state = wrong_id
        .path()
        .join("state-CLI_SESSION_WRONG_ID_STATE_SECRET");
    let (alpha_record, _) =
        save_session_summary(&wrong_state, "alpha", "incarnation-alpha", 1, 1, 0, 0);
    let donor_state = wrong_id.path().join("donor-state");
    let (beta_record, _) =
        save_session_summary(&donor_state, "beta", "incarnation-beta", 1, 1, 0, 0);
    fs::copy(beta_record, alpha_record).unwrap();

    for (state, forbidden) in [
        (
            &malformed_state,
            &[
                "CLI_SESSION_MALFORMED_STATE_SECRET",
                "CLI_SESSION_MALFORMED_RECORD_SECRET",
                "alpha",
            ][..],
        ),
        (
            &wrong_state,
            &[
                "CLI_SESSION_WRONG_ID_STATE_SECRET",
                "incarnation-beta",
                "beta",
            ][..],
        ),
    ] {
        for (arguments, json) in [
            (&["session", "alpha"][..], false),
            (&["session", "alpha", "--json"][..], true),
        ] {
            let output = session_command(OsStr::new("relative-config"), state.as_os_str())
                .args(arguments)
                .output()
                .unwrap();
            assert_session_error(&output, json, "Corrupt");
            assert_output_omits(&output, forbidden);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn session_rejects_unsafe_symlink_and_wrong_kind_roots_without_mutation() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let temporary = TestDirectory::new("session-invalid-roots");
    let config_root = temporary.path().join("missing-config");
    let target = temporary.path().join("target");
    private_directory(&target);

    let wrong_kind_base = temporary.path().join("wrong-kind-base");
    fs::create_dir(&wrong_kind_base).unwrap();
    fs::write(
        wrong_kind_base.join("machine-god"),
        b"CLI_SESSION_WRONG_KIND_SECRET",
    )
    .unwrap();

    let symlink_base = temporary.path().join("symlink-base");
    fs::create_dir(&symlink_base).unwrap();
    symlink(&target, symlink_base.join("machine-god")).unwrap();

    let unsafe_base = temporary.path().join("unsafe-base");
    let unsafe_root = unsafe_base.join("machine-god");
    fs::create_dir_all(&unsafe_root).unwrap();
    fs::set_permissions(&unsafe_root, fs::Permissions::from_mode(0o755)).unwrap();

    for state_base in [&wrong_kind_base, &symlink_base, &unsafe_base] {
        let output = session_command(config_root.as_os_str(), state_base.as_os_str())
            .args(["session", "alpha", "--json"])
            .output()
            .unwrap();
        assert_session_error(&output, true, "Unavailable");
        assert_output_omits(&output, &["CLI_SESSION_WRONG_KIND_SECRET"]);
    }

    assert_eq!(
        fs::read(wrong_kind_base.join("machine-god")).unwrap(),
        b"CLI_SESSION_WRONG_KIND_SECRET"
    );
    assert!(symlink_base.join("machine-god").is_symlink());
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&unsafe_root).unwrap().count(), 0);
    assert!(!config_root.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn session_stdout_failure_uses_the_global_fixed_diagnostic() {
    use std::fs::OpenOptions;
    use std::process::Stdio;

    let temporary = TestDirectory::new("session-stdout-failure");
    let state_base = temporary.path().join("state");
    save_session_summary(&state_base, "alpha", "incarnation-alpha", 1, 1, 0, 0);
    let full = OpenOptions::new().write(true).open("/dev/full").unwrap();
    let output = session_command(OsStr::new("relative-config"), state_base.as_os_str())
        .args(["session", "alpha"])
        .stdout(Stdio::from(full))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, OUTPUT_FAILURE.as_bytes());
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[test]
fn session_is_fixed_unsupported_on_other_targets() {
    for (arguments, json) in [
        (&["session", "alpha"][..], false),
        (&["session", "alpha", "--json"][..], true),
    ] {
        assert_session_error(&run(arguments), json, "Unsupported");
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn sessions_missing_root_is_exact_empty_and_ignores_unrelated_host_inputs() {
    let temporary = TestDirectory::new("sessions-missing");
    let config_root = temporary.path().join("config-CLI_SESSIONS_CONFIG_PATH");
    let state_root = temporary
        .path()
        .join("missing-state-CLI_SESSIONS_STATE_PATH");
    let contents = b"CLI_SESSIONS_INVALID_CONFIG_SECRET:not-json";
    let config_path = write_config(&config_root, contents);

    let human = sessions_command(config_root.as_os_str(), state_root.as_os_str())
        .arg("sessions")
        .output()
        .unwrap();
    assert_success(&human, "[sessions] no saved sessions\n");
    let json = sessions_command(config_root.as_os_str(), state_root.as_os_str())
        .args(["sessions", "--json"])
        .output()
        .unwrap();
    assert_success(
        &json,
        "{\"kind\":\"sessions\",\"count\":0,\"truncated\":false,\"sessions\":[]}\n",
    );

    assert_output_omits(
        &human,
        &[
            "CLI_SESSIONS_CONFIG_PATH",
            "CLI_SESSIONS_STATE_PATH",
            "CLI_SESSIONS_INVALID_CONFIG_SECRET",
            "CLI_SESSIONS_IGNORED_INVALID CREDENTIAL",
            "CLI_SESSIONS_IGNORED_LOWER_CREDENTIAL",
        ],
    );
    assert_output_omits(
        &json,
        &[
            "CLI_SESSIONS_CONFIG_PATH",
            "CLI_SESSIONS_STATE_PATH",
            "CLI_SESSIONS_INVALID_CONFIG_SECRET",
            "CLI_SESSIONS_IGNORED_INVALID CREDENTIAL",
            "CLI_SESSIONS_IGNORED_LOWER_CREDENTIAL",
        ],
    );
    assert_eq!(fs::read(config_path).unwrap(), contents);
    assert!(!state_root.exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn sessions_xdg_precedes_home_and_outputs_sorted_ids_exactly() {
    let temporary = TestDirectory::new("sessions-xdg-precedence");
    let xdg_state = temporary.path().join("xdg-state");
    let home = temporary.path().join("home");
    let home_state = home.join(".local/state");
    save_session(&xdg_state, "zeta-session");
    save_session(&xdg_state, "alpha-session");
    save_session(&home_state, "home-must-not-appear");

    let mut command = machine_god();
    command
        .arg("sessions")
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", "relative-config-must-not-be-read")
        .env("XDG_STATE_HOME", &xdg_state)
        .env("VERCEL_OIDC_TOKEN", "invalid credential must not be read");
    assert_success(
        &command.output().unwrap(),
        "[sessions] 2 saved\n - alpha-session\n - zeta-session\n",
    );

    let mut command = machine_god();
    command
        .args(["sessions", "--json"])
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", "relative-config-must-not-be-read")
        .env("XDG_STATE_HOME", &xdg_state)
        .env("VERCEL_OIDC_TOKEN", "invalid credential must not be read");
    assert_success(
        &command.output().unwrap(),
        concat!(
            "{\"kind\":\"sessions\",\"count\":2,\"truncated\":false,\"sessions\":[",
            "{\"id\":\"alpha-session\"},{\"id\":\"zeta-session\"}]}\n",
        ),
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn sessions_home_fallback_uses_dot_local_state() {
    let temporary = TestDirectory::new("sessions-home-fallback");
    let home = temporary.path().join("home");
    save_session(&home.join(".local/state"), "fallback-session");

    let output = machine_god()
        .args(["sessions", "--json"])
        .env("HOME", &home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_STATE_HOME")
        .env_remove("VERCEL_OIDC_TOKEN")
        .env_remove("AI_GATEWAY_API_KEY")
        .output()
        .unwrap();
    assert_success(
        &output,
        concat!(
            "{\"kind\":\"sessions\",\"count\":1,\"truncated\":false,",
            "\"sessions\":[{\"id\":\"fallback-session\"}]}\n",
        ),
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn sessions_listing_may_create_only_a_private_lock_sidecar() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = TestDirectory::new("sessions-lock-sidecar");
    let config_root = temporary.path().join("missing-config");
    let state_base = temporary.path().join("state");
    let state_root = state_base.join("machine-god");
    save_session(&state_base, "lock-session");

    let record_path = fs::read_dir(&state_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .unwrap();
    let lock_path = fs::read_dir(&state_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "lock")
        })
        .unwrap();
    fs::remove_file(&lock_path).unwrap();
    let record_before = fs::read(&record_path).unwrap();

    let output = sessions_command(config_root.as_os_str(), state_base.as_os_str())
        .arg("sessions")
        .output()
        .unwrap();
    assert_success(&output, "[sessions] 1 saved\n - lock-session\n");
    assert_eq!(fs::read(&record_path).unwrap(), record_before);

    let mut entries = fs::read_dir(&state_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    entries.sort();
    assert_eq!(entries.len(), 2);
    let recreated_lock = entries
        .iter()
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "lock")
        })
        .unwrap();
    assert_eq!(
        fs::metadata(recreated_lock).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(!config_root.exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn sessions_reports_bounded_incomplete_results() {
    let temporary = TestDirectory::new("sessions-truncated");
    let config_root = temporary.path().join("missing-config");
    let state_base = temporary.path().join("state");
    for index in 0..101 {
        save_session(&state_base, &format!("bounded-session-{index:03}"));
    }

    let human = sessions_command(config_root.as_os_str(), state_base.as_os_str())
        .arg("sessions")
        .output()
        .unwrap();
    assert!(human.status.success());
    assert!(human.stderr.is_empty());
    let human = String::from_utf8(human.stdout).unwrap();
    assert!(human.starts_with("[sessions] 100 saved\n"));
    assert_eq!(human.matches("\n - bounded-session-").count(), 100);
    assert!(human.ends_with("[sessions] listing incomplete: a resource limit was reached\n"));

    let json = sessions_command(config_root.as_os_str(), state_base.as_os_str())
        .args(["sessions", "--json"])
        .output()
        .unwrap();
    assert!(json.status.success());
    assert!(json.stderr.is_empty());
    let json = String::from_utf8(json.stdout).unwrap();
    assert!(
        json.starts_with("{\"kind\":\"sessions\",\"count\":100,\"truncated\":true,\"sessions\":[")
    );
    assert_eq!(json.matches("{\"id\":\"bounded-session-").count(), 100);
    assert!(json.ends_with("]}\n"));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn sessions_corrupt_record_is_fixed_and_redacted() {
    let temporary = TestDirectory::new("sessions-corrupt");
    let config_root = temporary.path().join("missing-config");
    let state_base = temporary.path().join("state-CLI_SESSIONS_STATE_SECRET");
    let state_root = state_base.join("machine-god");
    save_session(&state_base, "corrupt-session");
    let record_path = fs::read_dir(&state_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .unwrap();
    fs::write(&record_path, b"CLI_SESSIONS_CORRUPT_RECORD_SECRET:not-json").unwrap();

    for (arguments, json) in [
        (&["sessions"][..], false),
        (&["sessions", "--json"][..], true),
    ] {
        let output = sessions_command(config_root.as_os_str(), state_base.as_os_str())
            .args(arguments)
            .output()
            .unwrap();
        assert_sessions_error(&output, json, "Corrupt");
        assert_output_omits(
            &output,
            &[
                "CLI_SESSIONS_STATE_SECRET",
                "CLI_SESSIONS_CORRUPT_RECORD_SECRET",
                "corrupt-session",
            ],
        );
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn sessions_rejects_unsafe_symlink_and_wrong_kind_roots_without_mutation() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let temporary = TestDirectory::new("sessions-invalid-roots");
    let config_root = temporary.path().join("missing-config");
    let target = temporary.path().join("target");
    private_directory(&target);

    let wrong_kind_base = temporary.path().join("wrong-kind-base");
    fs::create_dir(&wrong_kind_base).unwrap();
    fs::write(
        wrong_kind_base.join("machine-god"),
        b"CLI_SESSIONS_WRONG_KIND_SECRET",
    )
    .unwrap();

    let symlink_base = temporary.path().join("symlink-base");
    fs::create_dir(&symlink_base).unwrap();
    symlink(&target, symlink_base.join("machine-god")).unwrap();

    let unsafe_base = temporary.path().join("unsafe-base");
    let unsafe_root = unsafe_base.join("machine-god");
    fs::create_dir_all(&unsafe_root).unwrap();
    fs::set_permissions(&unsafe_root, fs::Permissions::from_mode(0o755)).unwrap();

    for (label, state_base) in [
        ("WRONG_KIND", &wrong_kind_base),
        ("SYMLINK", &symlink_base),
        ("UNSAFE", &unsafe_base),
    ] {
        let output = sessions_command(config_root.as_os_str(), state_base.as_os_str())
            .args(["sessions", "--json"])
            .output()
            .unwrap();
        assert_sessions_error(&output, true, "Unavailable");
        assert_output_omits(&output, &[label, "CLI_SESSIONS_WRONG_KIND_SECRET"]);
    }

    assert_eq!(
        fs::read(wrong_kind_base.join("machine-god")).unwrap(),
        b"CLI_SESSIONS_WRONG_KIND_SECRET"
    );
    assert!(symlink_base.join("machine-god").is_symlink());
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&unsafe_root).unwrap().count(), 0);
    assert!(!config_root.exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn invalid_sessions_grammar_precedes_corrupt_state_access() {
    let temporary = TestDirectory::new("sessions-argument-precedence");
    let config_root = temporary.path().join("missing-config");
    let state_base = temporary.path().join("state");
    let state_root = state_base.join("machine-god");
    save_session(&state_base, "argument-precedence");
    let record_path = fs::read_dir(&state_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .unwrap();
    let contents = b"CLI_SESSIONS_ARGUMENT_PRECEDENCE_SECRET:not-json";
    fs::write(&record_path, contents).unwrap();

    for arguments in [
        &["sessions", "extra"][..],
        &["sessions", "--json=true"][..],
        &["sessions", "--json", "extra"][..],
        &["sessions", "--json", "--json"][..],
    ] {
        let output = sessions_command(config_root.as_os_str(), state_base.as_os_str())
            .args(arguments)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, INVALID_ARGUMENTS.as_bytes());
    }
    assert_eq!(fs::read(record_path).unwrap(), contents);
    assert!(!config_root.exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn sessions_invalid_state_environment_is_fixed_without_home_fallback() {
    let temporary = TestDirectory::new("sessions-invalid-environment");
    let home = temporary.path().join("home");
    save_session(&home.join(".local/state"), "home-must-not-be-listed");

    for (arguments, json) in [
        (&["sessions"][..], false),
        (&["sessions", "--json"][..], true),
    ] {
        let output = machine_god()
            .args(arguments)
            .env("HOME", &home)
            .env("XDG_STATE_HOME", "CLI_SESSIONS_RELATIVE_STATE_SECRET")
            .env_remove("XDG_CONFIG_HOME")
            .output()
            .unwrap();
        assert_sessions_error(&output, json, "Unavailable");
        assert_output_omits(
            &output,
            &[
                "CLI_SESSIONS_RELATIVE_STATE_SECRET",
                "home-must-not-be-listed",
            ],
        );
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn sessions_non_unicode_state_environment_is_fixed_and_redacted() {
    use std::os::unix::ffi::OsStringExt;

    let temporary = TestDirectory::new("sessions-non-unicode-environment");
    let home = temporary.path().join("home");
    save_session(&home.join(".local/state"), "home-must-not-be-listed");
    let invalid = OsString::from_vec(b"CLI_SESSIONS_NON_UNICODE_STATE_SECRET-\xff".to_vec());

    let output = machine_god()
        .args(["sessions", "--json"])
        .env("HOME", &home)
        .env("XDG_STATE_HOME", invalid)
        .env_remove("XDG_CONFIG_HOME")
        .output()
        .unwrap();
    assert_sessions_error(&output, true, "Unavailable");
    assert_output_omits(
        &output,
        &[
            "CLI_SESSIONS_NON_UNICODE_STATE_SECRET",
            "home-must-not-be-listed",
        ],
    );
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[test]
fn sessions_is_fixed_unsupported_on_other_targets() {
    for (arguments, json) in [
        (&["sessions"][..], false),
        (&["sessions", "--json"][..], true),
    ] {
        assert_sessions_error(&run(arguments), json, "Unsupported");
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
        vec![OsString::from("session"), OsString::from_vec(vec![0xff])],
        vec![
            OsString::from("session"),
            OsString::from("alpha"),
            OsString::from_vec(vec![0xff]),
        ],
        vec![OsString::from("sessions"), OsString::from_vec(vec![0xff])],
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
