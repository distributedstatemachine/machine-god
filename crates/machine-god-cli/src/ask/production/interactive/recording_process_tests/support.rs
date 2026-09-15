use rustix::{
    fs::{Mode, OFlags},
    process::{Pid, Signal},
};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        process::CommandExt,
    },
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
    executable: PathBuf,
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
        #[cfg(target_os = "macos")]
        let executable = root.join("bin/recording-process-child");
        // Relocation is a macOS-only workaround. On Linux a cross-filesystem
        // copy opens a new executable for writing; concurrent spawned children
        // can inherit that descriptor until exec and cause ETXTBSY even after
        // fs::copy returns. Reuse the already-running executable instead.
        #[cfg(not(target_os = "macos"))]
        let executable = std::env::current_exe().unwrap();
        let fixture = Self {
            root,
            workspace,
            state,
            configuration,
            executable,
        };
        #[cfg(target_os = "macos")]
        {
            // CoreFoundation discovers the executable's bundle while production
            // MCP captures system DNS. Cargo's deps directory can contain an
            // unbounded build history; never make that scan part of this fixture.
            // Keep the exact executable, not a symlink back into Cargo's tree.
            // Install cleanup ownership before staging can fail.
            let bin = fixture.executable.parent().unwrap();
            fs::create_dir(bin).unwrap();
            fs::set_permissions(bin, fs::Permissions::from_mode(0o700)).unwrap();
            stage_executable(
                &std::env::current_exe().unwrap(),
                &fixture.executable,
                |source, destination| fs::hard_link(source, destination),
            )
            .unwrap();
        }
        fixture
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
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
        // Scenario owners are declared after this fixture and settle their
        // children before it drops, including unwinding through OwnedChild.
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[cfg(target_os = "macos")]
fn stage_executable(
    source: &Path,
    destination: &Path,
    link: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    let original = fs::symlink_metadata(source)?;
    if !original.is_file() || original.permissions().mode() & 0o111 == 0 {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let linked = match link(source, destination) {
        Ok(()) => true,
        // A private temp root may be on another filesystem. Copy only in that
        // explicit case; other staging failures must remain test failures.
        Err(error) if error.raw_os_error() == Some(rustix::io::Errno::XDEV.raw_os_error()) => {
            if fs::copy(source, destination)? != original.len() {
                return Err(io::ErrorKind::InvalidData.into());
            }
            false
        }
        Err(error) => return Err(error),
    };
    let staged = fs::symlink_metadata(destination)?;
    if !staged.is_file()
        || staged.len() != original.len()
        || staged.permissions().mode() != original.permissions().mode()
        || (linked && (staged.dev() != original.dev() || staged.ino() != original.ino()))
    {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(())
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
                "CLI exited before expected output {:?}: {}",
                String::from_utf8_lossy(bytes),
                String::from_utf8_lossy(&self.output)
            );
            assert!(
                Instant::now() < deadline,
                "CLI output deadline waiting for {:?}: {}",
                String::from_utf8_lossy(bytes),
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

    /// Physical master closure is hangup, not a Ctrl-D gesture or clean EOF.
    pub fn hangup(self) -> ExitStatus {
        let Self {
            master,
            slave,
            mut child,
            settings,
            ..
        } = self;
        drop(master);
        let deadline = Instant::now() + DEADLINE;
        let status = loop {
            if let Some(status) = child.poll() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "CLI did not settle physical PTY hangup"
            );
            std::thread::sleep(Duration::from_millis(2));
        };
        match rustix::termios::tcgetattr(slave.as_ref().unwrap()) {
            Ok(restored) => assert_eq!(format!("{restored:?}"), settings),
            // Some PTYs reject termios queries once their master has gone.
            Err(rustix::io::Errno::IO) => {}
            Err(error) => panic!("unexpected hangup termios error: {error}"),
        }
        status
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

#[test]
#[cfg(not(target_os = "macos"))]
fn recording_child_reuses_current_executable_without_staging() {
    let fixture = Fixture::new();
    let executable = std::env::current_exe().unwrap();
    assert_eq!(fixture.executable, executable);
    assert_eq!(fixture.command().get_program(), executable.as_os_str());
    let original = fs::metadata(&executable).unwrap();
    let selected = fs::metadata(fixture.command().get_program()).unwrap();
    assert_eq!(
        (selected.dev(), selected.ino()),
        (original.dev(), original.ino())
    );
    assert!(!executable.starts_with(&fixture.root));
    let mut entries = fs::read_dir(&fixture.root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    entries.sort();
    assert_eq!(
        entries,
        ["configuration", "state", "workspace"].map(std::ffi::OsString::from)
    );
}

#[test]
#[cfg(target_os = "macos")]
fn recording_child_uses_only_the_private_staged_executable_directory() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.command().get_program(),
        fixture.executable.as_os_str()
    );
    let bin = fixture.executable.parent().unwrap();
    assert_eq!(bin, fixture.root.join("bin"));
    for selected in [&fixture.workspace, &fixture.state, &fixture.configuration] {
        assert!(!bin.starts_with(selected));
    }
    assert_eq!(fs::read_dir(bin).unwrap().count(), 1);
    assert!(fs::symlink_metadata(&fixture.executable).unwrap().is_file());
    let original = fs::metadata(std::env::current_exe().unwrap()).unwrap();
    let staged = fs::metadata(&fixture.executable).unwrap();
    assert_eq!(staged.len(), original.len());
    assert_eq!(staged.permissions().mode(), original.permissions().mode());
}

#[test]
#[cfg(target_os = "macos")]
fn executable_staging_links_exact_identity_and_copies_only_across_filesystems() {
    let fixture = Fixture::new();
    let source = fixture.path("source");
    fs::write(&source, b"exact executable fixture bytes\0\xff").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o751)).unwrap();
    let linked = fixture.path("linked");
    stage_executable(&source, &linked, |source, destination| {
        fs::hard_link(source, destination)
    })
    .unwrap();
    let original = fs::metadata(&source).unwrap();
    let staged = fs::metadata(&linked).unwrap();
    assert_eq!(
        (staged.dev(), staged.ino()),
        (original.dev(), original.ino())
    );
    assert_eq!(fs::read(&linked).unwrap(), fs::read(&source).unwrap());

    let copied = fixture.path("copied");
    stage_executable(&source, &copied, |_, _| Err(rustix::io::Errno::XDEV.into())).unwrap();
    let staged = fs::metadata(&copied).unwrap();
    assert_ne!(staged.ino(), original.ino());
    assert_eq!(staged.permissions().mode(), original.permissions().mode());
    assert_eq!(fs::read(&copied).unwrap(), fs::read(&source).unwrap());

    let rejected = fixture.path("rejected");
    assert_eq!(
        stage_executable(&source, &rejected, |_, _| Err(
            io::ErrorKind::PermissionDenied.into()
        ))
        .unwrap_err()
        .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(!rejected.exists());
}

#[test]
#[cfg(target_os = "macos")]
fn executable_staging_rejects_symlinks_nonregular_and_nonexecutable_sources() {
    let fixture = Fixture::new();
    let source = fixture.path("source");
    fs::write(&source, b"executable fixture").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();
    let symlink = fixture.path("source-link");
    std::os::unix::fs::symlink(&source, &symlink).unwrap();
    let destination = fixture.path("destination");
    for invalid in [&symlink, &fixture.workspace] {
        assert_eq!(
            stage_executable(invalid, &destination, |_, _| panic!(
                "invalid source must not link"
            ))
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(!destination.exists());
    }
    fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        stage_executable(&source, &destination, |_, _| panic!(
            "nonexecutable source must not link"
        ))
        .unwrap_err()
        .kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(!destination.exists());
}
