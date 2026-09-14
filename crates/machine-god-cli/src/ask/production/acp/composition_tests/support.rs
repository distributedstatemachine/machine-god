use machine_god_native::{NativeInteractiveInputHelper, NativeInteractiveInputSource};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    fs::{self, File},
    io::{PipeReader, PipeWriter, Write},
    os::{
        fd::OwnedFd,
        unix::fs::{DirBuilderExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread::JoinHandle,
    time::Instant,
};

pub(super) const KEY: &str = "acp-local-composition-fixture-key";
static NEXT: AtomicU64 = AtomicU64::new(0);
const SENTINEL: &[u8] = b"invalid profile MCP sentinel: must remain unread";

pub(super) struct Fixture {
    root: PathBuf,
    pub workspace: PathBuf,
    config: PathBuf,
    state: PathBuf,
    config_bytes: Vec<u8>,
}
impl Fixture {
    pub fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "mg-acp-composition-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let workspace = root.join("workspace");
        let config = root.join("config");
        let state = root.join("state");
        for path in [&workspace, &config, &state] {
            fs::DirBuilder::new().mode(0o700).create(path).unwrap();
        }
        let profile = config.join("machine-god");
        fs::DirBuilder::new().mode(0o700).create(&profile).unwrap();
        let config_bytes = serde_json::to_vec(&json!({
            "schema_version":7,"permission_mode":"ask","sandbox_mode":"none",
            "permission_rules":[],"workspace_permission_rules":[],"workspace_directories":[],
            "provider":"vercel_ai_gateway","transport":"ai_gateway_http",
            "credential_source":"environment","model":"zai/glm-5.2","effort":"auto","fast_mode":false
        })).unwrap();
        fs::write(profile.join("config.json"), &config_bytes).unwrap();
        for name in ["mcp.json", "mcp-credentials.json"] {
            fs::write(profile.join(name), SENTINEL).unwrap();
        }
        Self {
            root,
            workspace,
            config,
            state,
            config_bytes,
        }
    }
    pub fn environment(&self) -> Vec<(OsString, OsString)> {
        // A caller-selected gate prerequisite, never an ambient credential or
        // a product endpoint/configuration switch. No fallback helper exists.
        let tmux = PathBuf::from(
            std::env::var_os("MACHINE_GOD_TERMINAL_TMUX_BINARY")
                .expect("the canonical gate must select its tmux prerequisite"),
        );
        assert!(tmux.is_absolute() && tmux.is_file());
        let paths = [
            tmux.parent().unwrap(),
            Path::new("/usr/bin"),
            Path::new("/bin"),
            Path::new("/usr/sbin"),
            Path::new("/sbin"),
        ];
        vec![
            ("HOME".into(), self.root.clone().into()),
            ("XDG_CONFIG_HOME".into(), self.config.clone().into()),
            ("XDG_STATE_HOME".into(), self.state.clone().into()),
            ("PATH".into(), std::env::join_paths(paths).unwrap()),
            ("AI_GATEWAY_API_KEY".into(), KEY.into()),
        ]
    }
    pub fn session_root(&self) -> PathBuf {
        self.state.join("machine-god")
    }
    pub fn assert_profile_unchanged(&self) {
        let profile = self.config.join("machine-god");
        assert_eq!(
            fs::read(profile.join("config.json")).unwrap(),
            self.config_bytes
        );
        for name in ["mcp.json", "mcp-credentials.json"] {
            assert_eq!(fs::read(profile.join(name)).unwrap(), SENTINEL);
        }
        assert_eq!(fs::read_dir(profile).unwrap().count(), 3);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) fn release_helper() -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY")
            .expect("the canonical gate must select its fresh release CLI helper"),
    );
    assert!(path.is_absolute() && path.is_file());
    assert_ne!(fs::metadata(&path).unwrap().permissions().mode() & 0o111, 0);
    path
}
pub(super) fn input_source(reader: PipeReader, path: &Path) -> NativeInteractiveInputSource {
    let descriptor: OwnedFd = reader.into();
    NativeInteractiveInputSource::PreserveSharedStream {
        input: File::from(descriptor),
        helper: NativeInteractiveInputHelper::new(path, File::open(path).unwrap()).unwrap(),
        null_device: None,
    }
}

pub(super) struct JoinedGuardian(pub Option<JoinHandle<()>>);
impl Drop for JoinedGuardian {
    fn drop(&mut self) {
        if let Some(worker) = self.0.take() {
            let result = worker.join();
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }
}

pub(super) struct Client {
    reader: PipeReader,
    writer: Option<PipeWriter>,
    deadline: Instant,
    bytes: usize,
    frames: usize,
}
impl Client {
    pub fn new(reader: PipeReader, writer: PipeWriter, deadline: Instant) -> Self {
        Self {
            reader,
            writer: Some(writer),
            deadline,
            bytes: 0,
            frames: 0,
        }
    }
    pub fn send(&mut self, id: u64, method: &str, params: Value) {
        let mut bytes =
            serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
                .unwrap();
        bytes.push(b'\n');
        // Only one tiny request is outstanding; its previous response proves
        // consumption before the next write. These cannot fill the pipe.
        assert!(bytes.len() <= 1024 && Instant::now() < self.deadline);
        self.writer.as_mut().unwrap().write_all(&bytes).unwrap();
    }
    pub fn response(&mut self, id: u64) -> (Value, Vec<Value>) {
        let mut updates = Vec::new();
        loop {
            let frame = self.frame().expect("response before EOF");
            if frame.get("id").is_some() {
                assert_eq!(frame["id"], id);
                assert!(frame.get("error").is_none(), "ACP response failed: {frame}");
                return (frame["result"].clone(), updates);
            }
            assert_eq!(frame["method"], "session/update");
            updates.push(frame["params"].clone());
        }
    }
    pub fn eof(&mut self) {
        drop(self.writer.take());
    }
    pub fn drain_to_eof(&mut self) {
        while let Some(frame) = self.frame() {
            assert_eq!(frame["method"], "session/update");
        }
    }
    fn frame(&mut self) -> Option<Value> {
        let mut frame = Vec::new();
        loop {
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "ACP composition response deadline elapsed"
            );
            // This is the only reader. Read one byte only after readable/HUP
            // readiness; no blocking reader thread can survive this deadline.
            let mut descriptors = [PollFd::new(&self.reader, PollFlags::IN)];
            match poll(
                &mut descriptors,
                Some(&Timespec::try_from(remaining).unwrap()),
            ) {
                Err(rustix::io::Errno::INTR) => continue,
                Ok(0) => panic!("ACP composition response deadline elapsed"),
                result => {
                    result.unwrap();
                    assert!(
                        descriptors[0]
                            .revents()
                            .intersects(PollFlags::IN | PollFlags::HUP)
                    );
                }
            }
            let mut byte = [0];
            match rustix::io::read(&self.reader, &mut byte) {
                Err(rustix::io::Errno::INTR) => continue,
                Ok(0) => {
                    assert!(frame.is_empty(), "truncated output");
                    return None;
                }
                result => assert_eq!(result.unwrap(), 1),
            }
            self.bytes += 1;
            assert!(self.bytes <= 256 * 1024 && frame.len() < 64 * 1024);
            if byte[0] == b'\n' {
                self.frames += 1;
                assert!(self.frames <= 128);
                return Some(serde_json::from_slice(&frame).unwrap());
            }
            frame.push(byte[0]);
        }
    }
}
