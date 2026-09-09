use rustix::fd::{AsFd, BorrowedFd};
use std::fmt::Debug;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) struct Fixture(pub(crate) PathBuf);

impl Fixture {
    pub(crate) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "mg-retained-root-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    pub(crate) fn root(&self) -> PathBuf {
        self.0.join("root")
    }

    pub(crate) fn open_root(&self) -> File {
        fs::create_dir(self.root()).unwrap();
        File::open(self.root()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

pub(crate) fn assert_root_states<E: Debug + Eq>(
    mut observe: impl FnMut(BorrowedFd<'_>) -> Result<(), E>,
    unavailable: &E,
) {
    let fixture = Fixture::new();
    let root = fixture.open_root();
    observe(root.as_fd()).unwrap();
    let renamed = fixture.0.join("renamed");
    fs::rename(fixture.root(), &renamed).unwrap();
    observe(root.as_fd()).unwrap();
    // A replacement at the original spelling does not replace the retained object.
    fs::create_dir(fixture.root()).unwrap();
    observe(root.as_fd()).unwrap();
    fs::remove_dir(&renamed).unwrap();
    assert_eq!(&observe(root.as_fd()).unwrap_err(), unavailable);
    observe(File::open(Path::new("/")).unwrap().as_fd()).unwrap();
}

#[test]
fn retained_root_sentinel_basename_and_raw_bytes_are_preserved() {
    let fixture = Fixture::new();
    let root = fixture.open_root();
    let metadata = rustix::fs::fstat(&root).unwrap();
    assert!(
        super::RetainedRootObservation::new(root.as_fd(), &metadata, c"/")
            .unwrap()
            .is_none()
    );
    for path in [c"", c"/trailing/"] {
        assert!(super::RetainedRootObservation::new(root.as_fd(), &metadata, path).is_err());
    }
    let path = std::ffi::CString::new(b"/parent/non-utf8-\xff".as_slice()).unwrap();
    let observation = super::RetainedRootObservation::new(root.as_fd(), &metadata, &path)
        .unwrap()
        .unwrap();
    assert_eq!(observation.name.to_bytes(), b"non-utf8-\xff");
}

#[test]
fn retained_root_identity_requires_device_inode_and_directory_type() {
    let fixture = Fixture::new();
    let root = fixture.open_root();
    let metadata = rustix::fs::fstat(&root).unwrap();
    let path = rustix::fs::getpath(&root).unwrap();
    let observation = super::RetainedRootObservation::new(root.as_fd(), &metadata, &path)
        .unwrap()
        .unwrap();
    let parent = observation.open_parent().unwrap();
    let linked = observation.stat_link(&parent).unwrap();
    assert!(observation.matches(&linked));
    let mut different_device = linked;
    different_device.st_dev = different_device.st_dev.wrapping_add(1);
    assert!(!observation.matches(&different_device));
    let mut different_inode = linked;
    different_inode.st_ino = different_inode.st_ino.wrapping_add(1);
    assert!(!observation.matches(&different_inode));
    let file = File::create(fixture.0.join("file")).unwrap();
    let mut regular = rustix::fs::fstat(&file).unwrap();
    regular.st_dev = linked.st_dev;
    regular.st_ino = linked.st_ino;
    assert!(!observation.matches(&regular));
}

#[test]
fn retained_root_parent_flags_and_nonfollowing_stale_name_observation_are_preserved() {
    let fixture = Fixture::new();
    let root = fixture.open_root();
    let metadata = rustix::fs::fstat(&root).unwrap();
    let path = rustix::fs::getpath(&root).unwrap();
    let observation = super::RetainedRootObservation::new(root.as_fd(), &metadata, &path)
        .unwrap()
        .unwrap();
    let parent = observation.open_parent().unwrap();
    assert!(
        rustix::fs::fcntl_getfl(&parent)
            .unwrap()
            .contains(rustix::fs::OFlags::NONBLOCK)
    );
    assert!(
        rustix::io::fcntl_getfd(&parent)
            .unwrap()
            .contains(rustix::io::FdFlags::CLOEXEC)
    );
    let retained = fixture.0.join("retained");
    fs::rename(fixture.root(), &retained).unwrap();
    assert_eq!(
        observation.stat_link(&parent).unwrap_err(),
        rustix::io::Errno::NOENT
    );
    std::os::unix::fs::symlink(&retained, fixture.root()).unwrap();
    // Following this symlink would incorrectly match the retained directory.
    assert!(!observation.matches(&observation.stat_link(&parent).unwrap()));
}
