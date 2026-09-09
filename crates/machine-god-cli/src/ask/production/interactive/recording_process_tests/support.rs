use rustix::{
    fs::{Mode, OFlags},
    process::{Pid, Signal},
};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

pub(super) const LIMIT: usize = 256 * 1024;
const DEADLINE: Duration = Duration::from_secs(10);
static NEXT: AtomicU64 = AtomicU64::new(0);

pub(super) struct Fixture {
    root: PathBuf,
    pub workspace: PathBuf,
    pub state: PathBuf,
    configuration: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "mg-recording-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let workspace = root.join("workspace");
        let state = root.join("state");
        let configuration = root.join("configuration");
        for path in [&workspace, &state, &configuration] {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self {
            root,
            workspace,
            state,
            configuration,
        }
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "ask::production::interactive::recording_process_tests::recording_process_child",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ]);
        command
            .env_clear()
            .current_dir(&self.workspace)
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", &self.configuration)
            .env("XDG_STATE_HOME", &self.state)
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("TERM", "xterm-256color")
            .env("AI_GATEWAY_API_KEY", "recording-local-fixture-key");
        command.env("RECORDING_TEST_ROOT", &self.root);
        if let Some(tmux) = std::env::var_os("MACHINE_GOD_TERMINAL_TMUX_BINARY") {
            command.env("MACHINE_GOD_TERMINAL_TMUX_BINARY", tmux);
        }
        if let Some(helper) = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY") {
            command.env("MACHINE_GOD_TERMINAL_RELEASE_BINARY", helper);
        }
        command
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

pub(super) struct OwnedChild {
    child: Option<Child>,
    group: Pid,
}

impl OwnedChild {
    pub fn spawn(command: &mut Command) -> Self {
        let child = command.process_group(0).spawn().unwrap();
        let group = Pid::from_raw(i32::try_from(child.id()).unwrap()).unwrap();
        Self {
            child: Some(child),
            group,
        }
    }

    pub fn signal(&mut self) {
        assert!(self.poll().is_none(), "signal requires a live owned CLI");
        rustix::process::kill_process(self.group, Signal::INT).unwrap();
    }

    pub fn poll(&mut self) -> Option<ExitStatus> {
        let status = self.child.as_mut()?.try_wait().unwrap();
        if status.is_some() {
            self.child.take();
        }
        status
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // The separate process group was created by this fixture. No PID
            // discovery or signal to an unrelated terminal process is involved.
            let _ = rustix::process::kill_process_group(self.group, Signal::KILL);
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub(super) struct Terminal {
    master: File,
    slave: Option<File>,
    pub child: OwnedChild,
    pub output: Vec<u8>,
    settings: String,
}

impl Terminal {
    pub fn spawn(command: &mut Command) -> Self {
        let (master, slave) = pty();
        let mut settings = rustix::termios::tcgetattr(&slave).unwrap();
        settings
            .output_modes
            .remove(rustix::termios::OutputModes::OPOST);
        rustix::termios::tcsetattr(&slave, rustix::termios::OptionalActions::Now, &settings)
            .unwrap();
        rustix::termios::tcsetwinsize(
            &slave,
            rustix::termios::Winsize {
                ws_row: 24,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )
        .unwrap();
        let settings = format!("{:?}", rustix::termios::tcgetattr(&slave).unwrap());
        let child = OwnedChild::spawn(
            command
                .stdin(Stdio::from(slave.try_clone().unwrap()))
                .stdout(Stdio::from(slave.try_clone().unwrap()))
                .stderr(Stdio::from(slave.try_clone().unwrap())),
        );
        // Reusable Command configuration must not retain a parent-side slave.
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        Self {
            master,
            slave: Some(slave),
            child,
            output: Vec::new(),
            settings,
        }
    }

    /// At most 64 bounded reads per observation, never a read-to-end allocation.
    fn drain(&mut self) -> bool {
        let mut chunk = [0_u8; 4096];
        for _ in 0..64 {
            match self.master.read(&mut chunk) {
                Ok(0) => return true,
                Ok(count) => {
                    assert!(
                        self.output.len() + count <= LIMIT,
                        "bounded CLI output exceeded"
                    );
                    self.output.extend_from_slice(&chunk[..count]);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return false,
                Err(error)
                    if error.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) =>
                {
                    return true;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => panic!("owned PTY read failed: {error}"),
            }
        }
        false
    }

    pub fn wait_for(&mut self, bytes: &[u8]) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            self.drain();
            if self
                .output
                .windows(bytes.len())
                .any(|window| window == bytes)
            {
                return;
            }
            assert!(
                self.child.poll().is_none(),
                "CLI exited before expected output: {}",
                String::from_utf8_lossy(&self.output)
            );
            assert!(
                Instant::now() < deadline,
                "CLI output deadline: {}",
                String::from_utf8_lossy(&self.output)
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    pub fn send(&mut self, mut bytes: &[u8]) {
        let deadline = Instant::now() + DEADLINE;
        while !bytes.is_empty() {
            assert!(Instant::now() < deadline, "PTY input deadline");
            match self.master.write(bytes) {
                Ok(0) => panic!("PTY input made no progress"),
                Ok(count) => bytes = &bytes[count..],
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    self.drain();
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("owned PTY input failed: {error}"),
            }
        }
    }

    pub fn finish(mut self) -> (ExitStatus, Vec<u8>) {
        let deadline = Instant::now() + DEADLINE;
        let status = loop {
            self.drain();
            if let Some(status) = self.child.poll() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "CLI exit deadline: {}",
                String::from_utf8_lossy(&self.output)
            );
            std::thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(
            format!(
                "{:?}",
                rustix::termios::tcgetattr(self.slave.as_ref().unwrap()).unwrap()
            ),
            self.settings,
            "CLI must restore exact terminal settings"
        );
        self.slave.take();
        while !self.drain() {
            assert!(
                Instant::now() < deadline,
                "a CLI helper retained the PTY after parent exit"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        (status, self.output)
    }
}

fn pty() -> (File, File) {
    let master = rustix::fs::open(
        "/dev/ptmx",
        OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .unwrap();
    rustix::pty::grantpt(&master).unwrap();
    rustix::pty::unlockpt(&master).unwrap();
    #[cfg(target_os = "linux")]
    let slave = rustix::pty::ioctl_tiocgptpeer(
        &master,
        rustix::pty::OpenptFlags::RDWR
            | rustix::pty::OpenptFlags::NOCTTY
            | rustix::pty::OpenptFlags::CLOEXEC,
    )
    .unwrap();
    #[cfg(target_os = "macos")]
    let slave = {
        let name = rustix::pty::ptsname(&master, Vec::new()).unwrap();
        rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .unwrap()
    };
    (master.into(), slave.into())
}

pub(super) fn bounded_file(path: &Path) -> Vec<u8> {
    let mut file = File::open(path).unwrap();
    assert!(file.metadata().unwrap().len() <= LIMIT as u64);
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .unwrap();
    assert!(bytes.len() <= LIMIT);
    bytes
}
