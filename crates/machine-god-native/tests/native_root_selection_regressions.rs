#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::error::Error as _;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use machine_god_native::{
    NativeEnvironment, NativeRootSelection, PreparedNativeRoots, PreparedNativeRootsErrorKind,
};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new() -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "machine-god-root-regression-{}-{identifier}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    set_mode(&path, 0o700);
                    return Self { path };
                }
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

fn create_private_directory(path: &Path) {
    fs::create_dir(path).unwrap();
    set_mode(path, 0o700);
}

fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn xdg_selection(workspace: &Path, state_base: OsString) -> NativeRootSelection {
    NativeRootSelection::from_environment(
        &NativeEnvironment::new(None, Some(state_base), None),
        workspace,
    )
    .unwrap()
}

#[test]
fn newly_created_xdg_roots_are_exact_0700_under_an_owner_masking_umask() {
    const CHILD_ROOT: &str = "MACHINE_GOD_NATIVE_ROOT_UMASK_TEST_ROOT";

    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = PathBuf::from(root);
        let workspace = root.join("workspace");
        let state_base = root.join("state");
        let selection = xdg_selection(&workspace, state_base.clone().into_os_string());
        let prepared = PreparedNativeRoots::prepare(selection).unwrap();

        assert_eq!(prepared.state_root(), state_base.join("machine-god"));
        assert_eq!(
            fs::metadata(prepared.state_root())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(format!("{prepared:?}"), "PreparedNativeRoots { .. }");
        return;
    }

    let temporary = TemporaryDirectory::new();
    create_private_directory(&temporary.path().join("workspace"));
    create_private_directory(&temporary.path().join("state"));

    let status = Command::new("sh")
        .arg("-c")
        .arg(
            "umask 0777; exec \"$1\" --exact \
             newly_created_xdg_roots_are_exact_0700_under_an_owner_masking_umask",
        )
        .arg("machine-god-native-root-regression-test")
        .arg(std::env::current_exe().unwrap())
        .env(CHILD_ROOT, temporary.path())
        .status()
        .unwrap();
    assert!(status.success(), "restrictive-umask child failed: {status}");
}

#[test]
fn decorated_final_state_base_symlinks_are_rejected_without_creating_the_suffix() {
    let temporary = TemporaryDirectory::new();
    let workspace = temporary.path().join("workspace");
    let target = temporary.path().join("state-target");
    let link = temporary.path().join("state-link");
    create_private_directory(&workspace);
    create_private_directory(&target);
    symlink(&target, &link).unwrap();

    for decorated in [
        format!("{}/", link.display()),
        format!("{}/.", link.display()),
    ] {
        let selection = xdg_selection(&workspace, OsString::from(decorated));
        assert_eq!(format!("{selection:?}"), "NativeRootSelection { .. }");

        let error = PreparedNativeRoots::prepare(selection).unwrap_err();
        assert_eq!(error.kind(), PreparedNativeRootsErrorKind::StateBase);
        assert_eq!(
            format!("{error:?}"),
            "PreparedNativeRootsError { kind: StateBase }"
        );
        assert_eq!(error.to_string(), "native state base preparation failed");
        assert!(error.source().is_none());
        assert!(!target.join("machine-god").exists());
    }
}
