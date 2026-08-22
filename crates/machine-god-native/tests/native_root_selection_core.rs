#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

use machine_god_native::{
    NativeEnvironment, NativeRootSelection, NativeRootSelectionErrorKind, PreparedNativeRoots,
    PreparedNativeRootsErrorKind,
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
                "mg-native-roots-{}-{identifier}",
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

fn environment(xdg_state_home: Option<&OsStr>, home: Option<&OsStr>) -> NativeEnvironment {
    NativeEnvironment::new(
        None,
        xdg_state_home.map(OsStr::to_os_string),
        home.map(OsStr::to_os_string),
    )
}

fn select(workspace: &Path, state_base: &Path) -> NativeRootSelection {
    NativeRootSelection::from_environment(
        &environment(Some(state_base.as_os_str()), None),
        workspace,
    )
    .expect("test roots select")
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}

fn set_mode(path: &Path, value: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(value)).unwrap();
}

fn create_private_dir(path: &Path) {
    fs::create_dir(path).unwrap();
    set_mode(path, 0o700);
}

#[test]
fn selection_uses_xdg_precedence_and_empty_xdg_falls_back_to_home() {
    let temporary = TemporaryDirectory::new();
    let workspace = temporary.path().join("workspace-that-need-not-exist");
    let xdg = temporary.path().join("xdg-that-need-not-exist");
    let home = temporary.path().join("home-that-need-not-exist");

    let selected = NativeRootSelection::from_environment(
        &environment(Some(xdg.as_os_str()), Some(home.as_os_str())),
        &workspace,
    )
    .unwrap();
    assert_eq!(selected.workspace_root(), workspace);
    assert_eq!(selected.state_root(), xdg.join("machine-god"));

    let fallback = NativeRootSelection::from_environment(
        &environment(Some(OsStr::new("")), Some(home.as_os_str())),
        &workspace,
    )
    .unwrap();
    assert_eq!(
        fallback.state_root(),
        home.join(".local").join("state").join("machine-god")
    );

    assert!(
        !workspace.exists(),
        "selection must not create the workspace"
    );
    assert!(!xdg.exists(), "selection must not create the XDG base");
    assert!(!home.exists(), "selection must not create the HOME base");
}

#[test]
fn selection_rejects_invalid_selected_values_without_fallback_or_writes() {
    let temporary = TemporaryDirectory::new();
    let workspace = temporary.path().join("missing-workspace");
    let home = temporary.path().join("valid-but-lower-precedence-home");
    let invalid_xdg = Path::new("relative-xdg");

    let invalid = NativeRootSelection::from_environment(
        &environment(Some(invalid_xdg.as_os_str()), Some(home.as_os_str())),
        &workspace,
    )
    .unwrap_err();
    assert_eq!(
        invalid.kind(),
        NativeRootSelectionErrorKind::InvalidStateEnvironment
    );
    assert!(!workspace.exists());
    assert!(!home.exists());

    let unavailable =
        NativeRootSelection::from_environment(&environment(None, Some(OsStr::new(""))), &workspace)
            .unwrap_err();
    assert_eq!(
        unavailable.kind(),
        NativeRootSelectionErrorKind::StateRootUnavailable
    );

    let invalid_home = NativeRootSelection::from_environment(
        &environment(None, Some(OsStr::new("relative-home"))),
        &workspace,
    )
    .unwrap_err();
    assert_eq!(
        invalid_home.kind(),
        NativeRootSelectionErrorKind::InvalidStateEnvironment
    );
}

#[test]
fn selection_accepts_non_unicode_workspace_but_rejects_non_unicode_state_and_bad_workspace() {
    let temporary = TemporaryDirectory::new();
    let invalid_unicode = OsString::from_vec(vec![b's', b'e', b'c', b'r', b'e', b't', 0xff]);
    let non_unicode_workspace = Path::new("/tmp").join(&invalid_unicode);
    NativeRootSelection::from_environment(
        &environment(Some(temporary.path().as_os_str()), None),
        &non_unicode_workspace,
    )
    .expect("workspace paths are operating-system paths");

    let non_unicode_state = Path::new("/tmp").join(&invalid_unicode);
    let state_error = NativeRootSelection::from_environment(
        &environment(Some(non_unicode_state.as_os_str()), None),
        temporary.path(),
    )
    .unwrap_err();
    assert_eq!(
        state_error.kind(),
        NativeRootSelectionErrorKind::InvalidStateEnvironment
    );

    for workspace in [Path::new("relative"), Path::new("/tmp/../workspace")] {
        let error = NativeRootSelection::from_environment(
            &environment(Some(temporary.path().as_os_str()), None),
            workspace,
        )
        .unwrap_err();
        assert_eq!(
            error.kind(),
            NativeRootSelectionErrorKind::InvalidWorkspaceRoot
        );
    }
}

#[test]
fn preparation_opens_workspace_first_and_never_creates_a_missing_state_base() {
    let temporary = TemporaryDirectory::new();
    let missing_workspace = temporary.path().join("missing-workspace");
    let existing_base = temporary.path().join("existing-state-base");
    create_private_dir(&existing_base);

    let workspace_error =
        PreparedNativeRoots::prepare(select(&missing_workspace, &existing_base)).unwrap_err();
    assert_eq!(
        workspace_error.kind(),
        PreparedNativeRootsErrorKind::WorkspaceRoot
    );
    assert!(!existing_base.join("machine-god").exists());

    let workspace = temporary.path().join("workspace");
    let missing_base = temporary.path().join("missing-state-base");
    create_private_dir(&workspace);
    let base_error = PreparedNativeRoots::prepare(select(&workspace, &missing_base)).unwrap_err();
    assert_eq!(base_error.kind(), PreparedNativeRootsErrorKind::StateBase);
    assert!(!missing_base.exists());
}

#[test]
fn preparation_creates_only_fixed_private_suffixes_and_does_not_repair_existing_modes() {
    let temporary = TemporaryDirectory::new();
    let workspace = temporary.path().join("workspace");
    let xdg = temporary.path().join("xdg");
    create_private_dir(&workspace);
    create_private_dir(&xdg);

    let prepared = PreparedNativeRoots::prepare(select(&workspace, &xdg)).unwrap();
    let state_root = xdg.join("machine-god");
    assert_eq!(prepared.workspace_root(), workspace);
    assert_eq!(prepared.state_root(), state_root);
    assert_eq!(mode(&state_root), 0o700);
    assert_eq!(
        fs::read_dir(&xdg)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>(),
        vec![OsString::from("machine-god")]
    );
    drop(prepared);

    set_mode(&state_root, 0o500);
    let prepared_again = PreparedNativeRoots::prepare(select(&workspace, &xdg)).unwrap();
    assert_eq!(mode(&state_root), 0o500);
    assert_eq!(prepared_again.state_root(), state_root);

    let home = temporary.path().join("home");
    create_private_dir(&home);
    let fallback = NativeRootSelection::from_environment(
        &environment(None, Some(home.as_os_str())),
        &workspace,
    )
    .unwrap();
    let fallback_root = home.join(".local/state/machine-god");
    PreparedNativeRoots::prepare(fallback).unwrap();
    assert_eq!(mode(&home.join(".local")), 0o700);
    assert_eq!(mode(&home.join(".local/state")), 0o700);
    assert_eq!(mode(&fallback_root), 0o700);
}

#[test]
fn preparation_rejects_wrong_kinds_symlinks_and_unsafe_existing_modes() {
    let temporary = TemporaryDirectory::new();
    let workspace = temporary.path().join("workspace");
    create_private_dir(&workspace);

    let workspace_file = temporary.path().join("workspace-file");
    fs::write(&workspace_file, b"not a directory").unwrap();
    let usable_base = temporary.path().join("usable-base");
    create_private_dir(&usable_base);
    let workspace_file_error =
        PreparedNativeRoots::prepare(select(&workspace_file, &usable_base)).unwrap_err();
    assert_eq!(
        workspace_file_error.kind(),
        PreparedNativeRootsErrorKind::WorkspaceRoot
    );
    let workspace_link = temporary.path().join("workspace-link");
    symlink(&workspace, &workspace_link).unwrap();
    let workspace_link_error =
        PreparedNativeRoots::prepare(select(&workspace_link, &usable_base)).unwrap_err();
    assert_eq!(
        workspace_link_error.kind(),
        PreparedNativeRootsErrorKind::WorkspaceRoot
    );
    assert!(!usable_base.join("machine-god").exists());

    let state_file = temporary.path().join("state-file");
    fs::write(&state_file, b"not a directory").unwrap();
    let file_error = PreparedNativeRoots::prepare(select(&workspace, &state_file)).unwrap_err();
    assert_eq!(file_error.kind(), PreparedNativeRootsErrorKind::StateBase);

    let real_base = temporary.path().join("real-base");
    let base_link = temporary.path().join("base-link");
    create_private_dir(&real_base);
    symlink(&real_base, &base_link).unwrap();
    let base_link_error = PreparedNativeRoots::prepare(select(&workspace, &base_link)).unwrap_err();
    assert_eq!(
        base_link_error.kind(),
        PreparedNativeRootsErrorKind::StateBase
    );

    let file_base = temporary.path().join("file-base");
    create_private_dir(&file_base);
    fs::write(file_base.join("machine-god"), b"not a directory").unwrap();
    let final_file_error =
        PreparedNativeRoots::prepare(select(&workspace, &file_base)).unwrap_err();
    assert_eq!(
        final_file_error.kind(),
        PreparedNativeRootsErrorKind::StateRoot
    );

    let link_base = temporary.path().join("link-base");
    create_private_dir(&link_base);
    symlink(&real_base, link_base.join("machine-god")).unwrap();
    let final_link_error =
        PreparedNativeRoots::prepare(select(&workspace, &link_base)).unwrap_err();
    assert_eq!(
        final_link_error.kind(),
        PreparedNativeRootsErrorKind::StateRoot
    );

    let unsafe_base = temporary.path().join("unsafe-base");
    create_private_dir(&unsafe_base);
    set_mode(&unsafe_base, 0o770);
    let unsafe_base_error =
        PreparedNativeRoots::prepare(select(&workspace, &unsafe_base)).unwrap_err();
    assert_eq!(
        unsafe_base_error.kind(),
        PreparedNativeRootsErrorKind::UnsafeStateDirectory
    );
    assert_eq!(mode(&unsafe_base), 0o770);

    let public_final_base = temporary.path().join("public-final-base");
    let public_final = public_final_base.join("machine-god");
    create_private_dir(&public_final_base);
    create_private_dir(&public_final);
    set_mode(&public_final, 0o750);
    let public_final_error =
        PreparedNativeRoots::prepare(select(&workspace, &public_final_base)).unwrap_err();
    assert_eq!(
        public_final_error.kind(),
        PreparedNativeRootsErrorKind::UnsafeStateDirectory
    );
    assert_eq!(mode(&public_final), 0o750);
}

#[cfg(target_os = "macos")]
#[test]
fn preparation_rejects_a_state_base_acl_without_creating_the_fixed_suffix() {
    let temporary = TemporaryDirectory::new();
    let workspace = temporary.path().join("workspace");
    let state_base = temporary.path().join("state-base-with-acl");
    create_private_dir(&workspace);
    create_private_dir(&state_base);

    let status = Command::new("/bin/chmod")
        .args(["+a", "everyone allow search"])
        .arg(&state_base)
        .status()
        .expect("macOS chmod executable is available");
    assert!(
        status.success(),
        "failed to install the ACL fixture: {status}"
    );
    assert_eq!(mode(&state_base), 0o700);

    let error = PreparedNativeRoots::prepare(select(&workspace, &state_base)).unwrap_err();
    assert_eq!(
        error.kind(),
        PreparedNativeRootsErrorKind::UnsafeStateDirectory
    );
    assert!(!state_base.join("machine-god").exists());
}

#[test]
fn preparation_rejects_equal_and_both_ancestor_directions_by_identity() {
    let temporary = TemporaryDirectory::new();

    let workspace = temporary.path().join("workspace");
    let state_base_in_workspace = workspace.join("state-base");
    create_private_dir(&workspace);
    create_private_dir(&state_base_in_workspace);
    let workspace_ancestor =
        PreparedNativeRoots::prepare(select(&workspace, &state_base_in_workspace)).unwrap_err();
    assert_eq!(
        workspace_ancestor.kind(),
        PreparedNativeRootsErrorKind::OverlappingRoots
    );
    assert!(!state_base_in_workspace.join("machine-god").exists());

    let equal_base = temporary.path().join("equal-base");
    let equal_root = equal_base.join("machine-god");
    create_private_dir(&equal_base);
    create_private_dir(&equal_root);
    set_mode(&equal_root, 0o700);
    let equal = PreparedNativeRoots::prepare(select(&equal_root, &equal_base)).unwrap_err();
    assert_eq!(equal.kind(), PreparedNativeRootsErrorKind::OverlappingRoots);

    let ancestor_base = temporary.path().join("state-ancestor-base");
    let state_ancestor = ancestor_base.join("machine-god");
    let workspace_below_state = state_ancestor.join("nested-workspace");
    create_private_dir(&ancestor_base);
    create_private_dir(&state_ancestor);
    create_private_dir(&workspace_below_state);
    set_mode(&state_ancestor, 0o700);
    let state_ancestor_error =
        PreparedNativeRoots::prepare(select(&workspace_below_state, &ancestor_base)).unwrap_err();
    assert_eq!(
        state_ancestor_error.kind(),
        PreparedNativeRootsErrorKind::OverlappingRoots
    );
}

#[test]
fn concurrent_preparers_share_the_same_private_fixed_root_under_normal_umask() {
    let temporary = TemporaryDirectory::new();
    let workspace = temporary.path().join("workspace");
    let state_base = temporary.path().join("state-base");
    create_private_dir(&workspace);
    create_private_dir(&state_base);

    let selection = Arc::new(select(&workspace, &state_base));
    let barrier = Arc::new(Barrier::new(8));
    let threads = (0..8)
        .map(|_| {
            let selection = Arc::clone(&selection);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                PreparedNativeRoots::prepare((*selection).clone())
            })
        })
        .collect::<Vec<_>>();

    for thread in threads {
        let prepared = thread
            .join()
            .expect("preparer thread does not panic")
            .unwrap();
        assert_eq!(prepared.state_root(), state_base.join("machine-god"));
    }
    assert_eq!(mode(&state_base.join("machine-god")), 0o700);
}

#[test]
fn selected_paths_and_all_public_diagnostics_are_stable_and_redacted() {
    let temporary = TemporaryDirectory::new();
    let workspace = temporary.path().join("secret-workspace");
    let state_base = temporary.path().join("secret-state-base");
    create_private_dir(&workspace);
    create_private_dir(&state_base);

    let selection = select(&workspace, &state_base);
    assert_eq!(format!("{selection:?}"), "NativeRootSelection { .. }");
    let prepared = PreparedNativeRoots::prepare(selection.clone()).unwrap();
    assert_eq!(prepared.selection(), &selection);
    assert_eq!(prepared.workspace_root(), workspace);
    assert_eq!(prepared.state_root(), state_base.join("machine-god"));
    assert_eq!(format!("{prepared:?}"), "PreparedNativeRoots { .. }");

    let selection_error = NativeRootSelection::from_environment(
        &environment(Some(OsStr::new("secret-relative-state")), None),
        Path::new("secret-relative-workspace"),
    )
    .unwrap_err();
    assert_eq!(
        selection_error.kind(),
        NativeRootSelectionErrorKind::InvalidWorkspaceRoot
    );
    assert_eq!(selection_error.kind().as_str(), "invalid_workspace_root");
    assert_eq!(
        selection_error.to_string(),
        "native workspace root selection is invalid"
    );
    assert_eq!(
        format!("{selection_error:?}"),
        "NativeRootSelectionError { kind: InvalidWorkspaceRoot }"
    );
    assert!(selection_error.source().is_none());

    let missing = temporary.path().join("secret-missing-workspace");
    let preparation_error =
        PreparedNativeRoots::prepare(select(&missing, &state_base)).unwrap_err();
    assert_eq!(
        preparation_error.kind(),
        PreparedNativeRootsErrorKind::WorkspaceRoot
    );
    assert_eq!(preparation_error.kind().as_str(), "workspace_root");
    assert_eq!(
        preparation_error.to_string(),
        "native workspace root preparation failed"
    );
    assert_eq!(
        format!("{preparation_error:?}"),
        "PreparedNativeRootsError { kind: WorkspaceRoot }"
    );
    assert!(preparation_error.source().is_none());
}
