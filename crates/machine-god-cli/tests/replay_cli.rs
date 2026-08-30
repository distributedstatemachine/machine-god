use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const REPLAY_HELP: &str = concat!(
    "machine-god replay\n",
    "\n",
    "Replay a recorded terminal session\n",
    "\n",
    "Usage:\n",
    "  machine-god replay <tape> [--frames] [--json] [--golden <path>] [--frames-dir <path>]\n",
    "\n",
    "Options:\n",
    "  --frames             Render each captured frame\n",
    "  --golden <path>      Write the final rendered grid to a file\n",
    "  --frames-dir <path>  Write rendered frames to a directory\n",
    "  --json               Emit machine-readable JSON instead of text\n",
);

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join("machine-god-replay-cli-tests");
        fs::create_dir_all(&root).unwrap();
        loop {
            let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!("{}-{name}-{id}", std::process::id()));
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
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn replay_command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_machine-god"));
    command
        .current_dir(root)
        .env("HOME", root.join("unrelated-home"))
        .env("XDG_CONFIG_HOME", root.join("unrelated-config"))
        .env("XDG_STATE_HOME", root.join("unrelated-state"))
        .env("AI_GATEWAY_API_KEY", "must-not-be-read")
        .env("VERCEL_OIDC_TOKEN", "must-not-be-read");
    command
}

fn run(root: &Path, arguments: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Output {
    replay_command(root).args(arguments).output().unwrap()
}

fn minimal_tape(cols: u16, rows: u16, payload: &[u8]) -> Vec<u8> {
    let version = b"vtest";
    let mut tape = Vec::new();
    tape.extend_from_slice(b"FXTP\x01");
    tape.extend_from_slice(&cols.to_le_bytes());
    tape.extend_from_slice(&rows.to_le_bytes());
    tape.extend_from_slice(&123_456_789_i64.to_le_bytes());
    tape.push(u8::try_from(version.len()).unwrap());
    tape.extend_from_slice(version);
    tape.extend_from_slice(&7_i32.to_le_bytes());
    tape.push(1); // stdout
    tape.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
    tape.extend_from_slice(payload);
    tape
}

#[test]
fn minimal_fxtp_tape_supports_default_frames_and_json_modes() {
    let root = TestDirectory::new("modes");
    let tape = root.path().join("minimal.fxtape");
    fs::write(&tape, minimal_tape(5, 2, b"hi")).unwrap();

    let default = run(root.path(), [OsStr::new("replay"), tape.as_os_str()]);
    assert_eq!(default.status.code(), Some(0));
    assert_eq!(default.stdout, b"|hi   |\n|     |\n");
    assert!(default.stderr.is_empty());

    let frames = run(
        root.path(),
        [
            OsStr::new("replay"),
            OsStr::new("--frames"),
            tape.as_os_str(),
        ],
    );
    assert_eq!(frames.status.code(), Some(0));
    assert_eq!(
        frames.stdout,
        b"\n--- frame 1 (stdout, +7ms) ---\n|hi   |\n|     |\n"
    );
    assert!(frames.stderr.is_empty());

    let json = run(
        root.path(),
        [OsStr::new("replay"), tape.as_os_str(), OsStr::new("--json")],
    );
    assert_eq!(json.status.code(), Some(0));
    assert_eq!(
        json.stdout,
        b"{\"cols\":5,\"rows\":2,\"epoch_ms\":123456789,\"version\":\"vtest\",\"frames\":[{\"delta_ms\":7,\"kind\":\"stdout\",\"len\":2}],\"frame_count\":1,\"resize_count\":0,\"stdout_bytes\":2}\n"
    );
    assert!(json.stderr.is_empty());
}

#[test]
fn golden_and_frames_directory_outputs_compose_without_unrelated_authority() {
    let root = TestDirectory::new("artifacts");
    let tape = root.path().join("minimal.fxtape");
    let golden = root.path().join("golden.txt");
    let frames = root.path().join("frame-artifacts");
    fs::write(&tape, minimal_tape(4, 1, b"ok")).unwrap();
    let config = root.path().join("unrelated-config/machine-god");
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("config.json"), b"not valid configuration").unwrap();

    let output = run(
        root.path(),
        [
            OsStr::new("replay"),
            OsStr::new("--golden"),
            OsStr::new("ignored.txt"),
            OsStr::new("--frames-dir"),
            OsStr::new("ignored-frames"),
            tape.as_os_str(),
            OsStr::new("--golden"),
            golden.as_os_str(),
            OsStr::new("--frames-dir"),
            frames.as_os_str(),
            OsStr::new("--json"),
        ],
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("\"frame_count\":1")
    );
    assert_eq!(fs::read(&golden).unwrap(), b"|ok  |\n");
    assert!(frames.join("manifest.json").is_file());
    assert!(frames.join("frames/0001.json").is_file());
    assert!(frames.join("frames/0001.grid.txt").is_file());
    assert!(!root.path().join("unrelated-state").exists());
}

#[test]
fn replay_help_anywhere_preempts_filesystem_effects() {
    let root = TestDirectory::new("help");
    for arguments in [
        vec!["replay", "--help"],
        vec!["replay", "missing.fxtape", "extra", "-h"],
        vec!["replay", "--golden", "--help"],
    ] {
        let output = run(root.path(), arguments);
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(output.stdout, REPLAY_HELP.as_bytes());
        assert!(output.stderr.is_empty());
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn replay_parse_errors_exit_one_and_raw_json_selects_structured_errors() {
    let root = TestDirectory::new("errors");
    let missing = run(root.path(), ["replay"]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(
        missing.stderr,
        concat!(
            "machine-god replay: missing tape path\n",
            "usage: machine-god replay <tape> [--frames] [--json] ",
            "[--golden <path>] [--frames-dir <path>]\n",
        )
        .as_bytes()
    );
    assert!(missing.stdout.is_empty());

    let structured = run(root.path(), ["replay", "tape", "extra", "--json"]);
    assert_eq!(structured.status.code(), Some(1));
    assert_eq!(
        structured.stdout,
        b"{\"kind\":\"replay\",\"error\":\"machine-god replay: too many positional arguments\",\"code\":\"TooManyArgs\"}\n"
    );
    assert!(structured.stderr.is_empty());

    let unknown = run(root.path(), ["replay", "tape", "--"]);
    assert_eq!(unknown.status.code(), Some(1));
    assert_eq!(unknown.stderr, b"machine-god replay: unknown flag\n");
}

#[test]
fn incomplete_tail_warning_is_machine_god_branded() {
    let root = TestDirectory::new("incomplete-tail");
    let tape = root.path().join("incomplete.fxtape");
    let mut bytes = minimal_tape(4, 1, b"ok");
    bytes.extend_from_slice(&[1, 2, 3]);
    fs::write(&tape, bytes).unwrap();

    let output = run(root.path(), [OsStr::new("replay"), tape.as_os_str()]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"|ok  |\n");
    assert_eq!(
        output.stderr,
        b"machine-god replay: ignored incomplete final tape frame\n"
    );
}
