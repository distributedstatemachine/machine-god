use super::*;
#[test]
fn config_lock_scope_releases_a_surviving_open_description_duplicate() {
    let directory =
        std::env::temp_dir().join(format!("mg-user-config-lock-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    let root = rustix::fs::open(&directory, READ | OFlags::DIRECTORY, Mode::empty()).unwrap();
    let survivor = {
        let original = open_lock(&root, ProfileFileKind::Settings).unwrap();
        let survivor = rustix::io::dup(&original).unwrap();
        let _guard = lock_config(original, ProfileFileKind::Settings).unwrap();
        let contender = open_lock(&root, ProfileFileKind::Settings).unwrap();
        assert!(matches!(
            lock_config(contender, ProfileFileKind::Settings),
            Err(ProfileFileError::Busy)
        ));
        survivor
    };
    let contender = open_lock(&root, ProfileFileKind::Settings).unwrap();
    let acquired = lock_config(contender, ProfileFileKind::Settings).unwrap();
    drop(acquired);
    drop(survivor);
    drop(root);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn config_lock_retries_interruption_but_not_busy_or_other_errors() {
    let mut calls = 0;
    let result = retry_lock_interrupted(ProfileFileKind::Settings, || {
        calls += 1;
        if calls < 3 {
            Err(rustix::io::Errno::INTR)
        } else {
            Ok(7)
        }
    });
    assert_eq!(result, Ok(7));
    assert_eq!(calls, 3);
    for error in [rustix::io::Errno::WOULDBLOCK, rustix::io::Errno::IO] {
        let mut calls = 0;
        let result: rustix::io::Result<()> =
            retry_lock_interrupted(ProfileFileKind::Settings, || {
                calls += 1;
                Err(error)
            });
        assert_eq!(result, Err(error));
        assert_eq!(calls, 1);
    }
}

#[test]
fn write_interruptions_are_bounded_even_with_partial_progress() {
    struct InterruptedWriter(usize);
    impl Write for InterruptedWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            self.0 += 1;
            if self.0.is_multiple_of(2) {
                Ok(1)
            } else {
                Err(std::io::ErrorKind::Interrupted.into())
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = InterruptedWriter(0);
    assert_eq!(
        write_bounded(&mut writer, &[0; 64]).unwrap_err().kind(),
        std::io::ErrorKind::Interrupted
    );
    assert_eq!(writer.0, 31);
}
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
pub(super) struct Fixture {
    pub(super) root: PathBuf,
}
impl Fixture {
    pub(super) fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "mg-profile-file-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            root: std::fs::canonicalize(root).unwrap(),
        }
    }
    pub(super) fn descriptor(&self) -> OwnedFd {
        rustix::fs::open(&self.root, READ | OFlags::DIRECTORY, Mode::empty()).unwrap()
    }
    pub(super) fn write(&self, name: &str, bytes: &[u8]) {
        std::fs::write(self.root.join(name), bytes).unwrap();
        std::fs::set_permissions(self.root.join(name), std::fs::Permissions::from_mode(0o600))
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn mcp_lock_interruptions_are_finite_for_acquisition_and_release() {
    let mut calls = 0;
    let result: rustix::io::Result<()> = retry_lock_interrupted(ProfileFileKind::Mcp, || {
        calls += 1;
        Err(rustix::io::Errno::INTR)
    });
    assert_eq!(result, Err(rustix::io::Errno::INTR));
    assert_eq!(calls, 16);
}

#[test]
fn mcp_guard_explicitly_unlocks_a_surviving_duplicate() {
    let fixture = Fixture::new();
    let root = fixture.descriptor();
    let fd = open_lock(&root, ProfileFileKind::Mcp).unwrap();
    let survivor = rustix::io::dup(&fd).unwrap();
    let guard = lock_config(fd, ProfileFileKind::Mcp).unwrap();
    assert!(matches!(
        lock_config(
            open_lock(&root, ProfileFileKind::Mcp).unwrap(),
            ProfileFileKind::Mcp
        ),
        Err(ProfileFileError::Busy)
    ));
    drop(guard);
    let guard = lock_config(
        open_lock(&root, ProfileFileKind::Mcp).unwrap(),
        ProfileFileKind::Mcp,
    )
    .unwrap();
    drop(guard);
    drop(survivor);
}

#[test]
fn bounded_reads_reject_limit_and_cumulative_interruptions() {
    struct Interrupted(usize);
    impl Read for Interrupted {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.0 += 1;
            if self.0.is_multiple_of(2) {
                buffer[0] = b'x';
                Ok(1)
            } else {
                Err(std::io::ErrorKind::Interrupted.into())
            }
        }
    }
    assert_eq!(
        read_bounded(&mut &b"1234"[..], 3),
        Err(ProfileFileError::TooLarge)
    );
    assert_eq!(read_bounded(&mut &b"123"[..], 3).unwrap(), b"123");
    let mut reader = Interrupted(0);
    assert_eq!(
        read_bounded(&mut reader, 64),
        Err(ProfileFileError::Unreadable)
    );
    assert_eq!(reader.0, 31);
}

#[test]
fn foreign_snapshot_cannot_cross_kind_or_store() {
    let fixture = Fixture::new();
    let settings = ProfileFile::new(fixture.root.clone(), ProfileFileKind::Settings);
    let mcp = ProfileFile::new(fixture.root.clone(), ProfileFileKind::Mcp);
    let observed = settings.observe().unwrap();
    assert_eq!(
        mcp.validate_unchanged(&observed),
        Err(ProfileFileError::Conflict)
    );
    assert!(matches!(
        mcp.begin(&observed, UpdateMode::CompareAndSwap),
        Err(ProfileFileError::Conflict)
    ));
    assert!(!fixture.root.join(".mcp.lock").exists());
}

#[test]
fn source_change_between_locked_observation_and_publication_is_rejected() {
    let fixture = Fixture::new();
    fixture.write("mcp.json", b"old");
    let store = ProfileFile::new(fixture.root.clone(), ProfileFileKind::Mcp);
    let observed = store.observe().unwrap();
    let transaction = store.begin(&observed, UpdateMode::CompareAndSwap).unwrap();
    fixture.write("mcp.json", b"changed");
    assert_eq!(
        transaction.publish(b"new", |_| Ok(())),
        Err(ProfileFileError::Conflict)
    );
    assert_eq!(
        std::fs::read(fixture.root.join("mcp.json")).unwrap(),
        b"changed"
    );
    assert!(!fixture.root.join(".mcp.tmp").exists());
}

#[test]
fn replaced_lock_and_foreign_temp_are_never_adopted() {
    for replace_lock in [true, false] {
        let fixture = Fixture::new();
        fixture.write("mcp.json", b"old");
        let store = ProfileFile::new(fixture.root.clone(), ProfileFileKind::Mcp);
        let observed = store.observe().unwrap();
        let transaction = store.begin(&observed, UpdateMode::CompareAndSwap).unwrap();
        if replace_lock {
            std::fs::rename(
                fixture.root.join(".mcp.lock"),
                fixture.root.join("old-lock"),
            )
            .unwrap();
            fixture.write(".mcp.lock", b"foreign");
        } else {
            fixture.write(".mcp.tmp", b"foreign");
        }
        assert!(transaction.publish(b"new", |_| Ok(())).is_err());
        assert_eq!(
            std::fs::read(fixture.root.join("mcp.json")).unwrap(),
            b"old"
        );
        let foreign = if replace_lock {
            ".mcp.lock"
        } else {
            ".mcp.tmp"
        };
        assert_eq!(
            std::fs::read(fixture.root.join(foreign)).unwrap(),
            b"foreign"
        );
    }
}
