use super::*;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    base: PathBuf,
    primary: PathBuf,
    state: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "machine-god-workspace-authority-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&base).unwrap();
        let base = std::fs::canonicalize(base).unwrap();
        let primary = base.join("primary");
        let state = base.join("state");
        std::fs::create_dir(&primary).unwrap();
        std::fs::create_dir(&state).unwrap();
        Self {
            base,
            primary,
            state,
        }
    }
    fn directory(&self, name: &str) -> PathBuf {
        let path = self.base.join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    fn authority(
        &self,
        specs: Vec<NativeWorkspaceEntrySpec>,
        suppressed: bool,
    ) -> Result<NativeWorkspaceAuthority> {
        NativeWorkspaceAuthority::open_blocking(
            open(&self.primary),
            self.primary.clone(),
            Some(open(&self.state)),
            self.state.clone(),
            specs,
            suppressed,
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.base).unwrap();
    }
}

fn open(path: &Path) -> OwnedFd {
    rustix::fs::open(path, DIRECTORY_FLAGS, Mode::empty()).unwrap()
}
fn spec(path: &Path, saved: bool, launch: bool) -> NativeWorkspaceEntrySpec {
    NativeWorkspaceEntrySpec::new(
        NativeWorkspaceSource::new(path.to_path_buf(), path.to_path_buf(), true).unwrap(),
        saved,
        launch,
    )
    .unwrap()
}

#[test]
fn provisional_symlink_source_acquires_identity_once_then_ignores_source_retarget() {
    let fixture = Fixture::new();
    let first = fixture.directory("first");
    let second = fixture.directory("second");
    let source = fixture.base.join("future-link");
    let provisional = NativeWorkspaceSource::new(source.clone(), source.clone(), false).unwrap();
    let authority = fixture
        .authority(
            vec![NativeWorkspaceEntrySpec::new(provisional, true, false).unwrap()],
            false,
        )
        .unwrap();
    assert!(!authority.snapshot().unwrap().entries()[0].available());
    std::os::unix::fs::symlink(&first, &source).unwrap();
    let installed = authority
        .install(authority.refresh_blocking().unwrap())
        .unwrap();
    assert!(installed.entries()[0].available() && installed.entries()[0].active());
    assert!(installed.entries()[0].source().identity_canonical());
    assert_eq!(installed.entries()[0].source().identity(), first);
    std::fs::remove_file(&source).unwrap();
    std::os::unix::fs::symlink(&second, &source).unwrap();
    let retained = authority
        .install(authority.refresh_blocking().unwrap())
        .unwrap();
    assert!(retained.entries()[0].active());
    assert_eq!(retained.entries()[0].source().identity(), first);
    assert!(retained.route(&second.join("file")).is_err());
}

#[test]
fn path_validation_is_pure_and_preserves_non_unicode() {
    let bytes = OsString::from_vec(vec![b'/', b'x', 0xff]);
    let source =
        NativeWorkspaceSource::new(PathBuf::from(&bytes), PathBuf::from(&bytes), true).unwrap();
    assert_eq!(source.source().as_os_str(), bytes);
    for invalid in [
        PathBuf::new(),
        PathBuf::from("/a/../b"),
        PathBuf::from(OsString::from_vec(vec![b'/', 0])),
        PathBuf::from(format!("/{}", "x".repeat(4096))),
    ] {
        assert_eq!(
            NativeWorkspaceSource::new(invalid.clone(), invalid, false),
            Err(NativeWorkspaceAuthorityError::InvalidPath)
        );
    }
    assert!(
        NativeWorkspaceSource::new(PathBuf::from("relative"), PathBuf::from("relative"), false)
            .is_err()
    );
}

#[test]
fn route_is_primary_first_boundary_safe_and_relative_primary_only() {
    let fixture = Fixture::new();
    let child = fixture.directory("primary/child");
    let extra = fixture.directory("extra");
    let authority = fixture
        .authority(
            vec![spec(&child, true, false), spec(&extra, false, true)],
            false,
        )
        .unwrap();
    let scope = authority.snapshot().unwrap();
    let route = scope.route(&child.join("file")).unwrap();
    assert_eq!(route.root_identity(), fixture.primary);
    assert_eq!(route.relative_path(), Path::new("child/file"));
    assert_eq!(
        scope
            .route(Path::new("extra/file"))
            .unwrap()
            .root_identity(),
        fixture.primary
    );
    assert_eq!(
        scope.route(&extra.join("./file")).unwrap().relative_path(),
        Path::new("file")
    );
    assert_eq!(
        scope.route(Path::new(".")).unwrap().relative_path(),
        Path::new(".")
    );
    for path in [
        fixture.base.join("extra-sibling/file"),
        fixture.state.join("data"),
        fixture.base.join("outside"),
        PathBuf::from("child/../escape"),
    ] {
        assert!(matches!(
            scope.route(&path),
            Err(NativeWorkspaceAuthorityError::InvalidPath)
        ));
    }
}

#[test]
fn suppression_preserves_provenance_launch_and_missing_entries() {
    let fixture = Fixture::new();
    let saved = fixture.directory("saved");
    let both = fixture.directory("both");
    let missing = fixture.base.join("missing");
    let authority = fixture
        .authority(
            vec![
                spec(&saved, true, false),
                spec(&both, true, true),
                spec(&missing, false, true),
            ],
            true,
        )
        .unwrap();
    let scope = authority.snapshot().unwrap();
    assert!(scope.saved_suppressed());
    assert!(scope.entries()[0].available());
    assert!(!scope.entries()[0].active());
    assert!(scope.entries()[1].active());
    assert!(scope.entries()[1].saved() && scope.entries()[1].launch());
    assert!(!scope.entries()[2].available());
    assert!(!scope.entries()[2].active());
    assert!(scope.route(&saved).is_err());
    assert!(scope.route(&both).is_ok());
}

#[test]
fn bounded_entries_count_inactive_and_suppressed() {
    let fixture = Fixture::new();
    let entries = (0..16)
        .map(|index| spec(&fixture.base.join(format!("missing{index}")), true, false))
        .collect::<Vec<_>>();
    assert_eq!(
        fixture
            .authority(entries.clone(), true)
            .unwrap()
            .snapshot()
            .unwrap()
            .entries()
            .len(),
        16
    );
    let mut too_many = entries;
    too_many.push(spec(&fixture.base.join("seventeenth"), false, true));
    assert!(matches!(
        fixture.authority(too_many, false),
        Err(NativeWorkspaceAuthorityError::TooManyDirectories)
    ));
}

#[test]
fn duplicate_primary_additional_and_state_overlap_are_rejected() {
    let fixture = Fixture::new();
    let extra = fixture.directory("extra");
    assert!(matches!(
        fixture.authority(vec![spec(&fixture.primary, true, false)], false),
        Err(NativeWorkspaceAuthorityError::DuplicateRoot)
    ));
    assert!(matches!(
        fixture.authority(
            vec![spec(&extra, true, false), spec(&extra, false, true)],
            false
        ),
        Err(NativeWorkspaceAuthorityError::DuplicateRoot)
    ));
    for path in [
        &fixture.state,
        &fixture.base,
        &fixture.state.join("missing"),
    ] {
        assert!(matches!(
            fixture.authority(vec![spec(path, true, false)], false),
            Err(NativeWorkspaceAuthorityError::OverlappingState)
        ));
    }
}

#[test]
fn descriptor_aliases_and_wrong_primary_descriptor_are_rejected() {
    let fixture = Fixture::new();
    let extra = fixture.directory("extra");
    let alias = fixture.base.join("alias");
    std::os::unix::fs::symlink(&fixture.base, &alias).unwrap();
    let source =
        NativeWorkspaceSource::new(alias.join("extra"), alias.join("extra"), false).unwrap();
    assert!(matches!(
        fixture.authority(
            vec![
                spec(&extra, true, false),
                NativeWorkspaceEntrySpec::new(source, false, true).unwrap()
            ],
            false
        ),
        Err(NativeWorkspaceAuthorityError::DuplicateRoot)
    ));
    let state_source =
        NativeWorkspaceSource::new(alias.join("state"), alias.join("state"), false).unwrap();
    assert!(matches!(
        fixture.authority(
            vec![NativeWorkspaceEntrySpec::new(state_source, true, false).unwrap()],
            false
        ),
        Err(NativeWorkspaceAuthorityError::OverlappingState)
    ));
    assert!(matches!(
        NativeWorkspaceAuthority::open_blocking(
            open(&extra),
            fixture.primary.clone(),
            Some(open(&fixture.state)),
            fixture.state.clone(),
            vec![],
            false
        ),
        Err(NativeWorkspaceAuthorityError::Unavailable)
    ));
}

#[test]
fn install_rejects_foreign_and_stale_preparations() {
    let fixture = Fixture::new();
    let first = fixture.authority(vec![], false).unwrap();
    let second = fixture.authority(vec![], false).unwrap();
    let foreign = first.prepare_blocking(vec![], true).unwrap();
    assert!(matches!(
        second.install(foreign),
        Err(NativeWorkspaceAuthorityError::WrongAuthority)
    ));
    let stale = first.prepare_blocking(vec![], true).unwrap();
    let accepted = first.prepare_blocking(vec![], false).unwrap();
    assert_eq!(accepted.snapshot().generation(), 1);
    assert_eq!(first.clone().install(accepted).unwrap().generation(), 1);
    assert!(matches!(
        first.install(stale),
        Err(NativeWorkspaceAuthorityError::StaleGeneration)
    ));
    assert!(!first.snapshot().unwrap().saved_suppressed());
}

#[test]
fn old_snapshots_and_routes_retain_exact_descriptors_after_install_and_rename() {
    let fixture = Fixture::new();
    let extra = fixture.directory("extra");
    std::fs::write(extra.join("marker"), b"old").unwrap();
    let authority = fixture
        .authority(vec![spec(&extra, true, false)], false)
        .unwrap();
    let old = authority.snapshot().unwrap();
    let route = old.route(&extra.join("marker")).unwrap();
    let metadata = rustix::fs::fstat(route.root_descriptor()).unwrap();
    std::fs::rename(&extra, fixture.base.join("moved")).unwrap();
    let new = authority
        .install(authority.prepare_blocking(vec![], false).unwrap())
        .unwrap();
    assert!(new.route(&extra).is_err());
    drop(authority);
    assert_eq!(route.generation(), 0);
    assert!(same_identity(
        &metadata,
        &rustix::fs::fstat(old.route(&extra).unwrap().root_descriptor()).unwrap()
    ));
    let file = rustix::fs::openat(
        route.root_descriptor(),
        route.relative_path(),
        OFlags::RDONLY | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .unwrap();
    let mut bytes = [0; 3];
    assert_eq!(rustix::io::read(file, &mut bytes).unwrap(), 3);
    assert_eq!(&bytes, b"old");
}

#[test]
fn canonical_identity_retarget_is_inactive_then_same_identity_reactivates() {
    let fixture = Fixture::new();
    let extra = fixture.directory("extra");
    let other = fixture.directory("other");
    let authority = fixture
        .authority(vec![spec(&extra, true, false)], false)
        .unwrap();
    let old = authority.snapshot().unwrap();
    std::fs::rename(&extra, fixture.base.join("moved")).unwrap();
    std::os::unix::fs::symlink(&other, &extra).unwrap();
    let unavailable = authority
        .install(authority.refresh_blocking().unwrap())
        .unwrap();
    assert_eq!(unavailable.entries()[0].source().identity(), extra);
    assert!(!unavailable.entries()[0].available());
    assert!(old.entries()[0].available());
    std::fs::remove_file(&extra).unwrap();
    std::fs::rename(fixture.base.join("moved"), &extra).unwrap();
    assert!(
        authority
            .install(authority.refresh_blocking().unwrap())
            .unwrap()
            .entries()[0]
            .active()
    );
}

#[test]
fn unavailable_noncanonical_source_acquires_and_retains_first_identity() {
    let fixture = Fixture::new();
    let target = fixture.directory("target");
    let alias = fixture.base.join("alias");
    let source = NativeWorkspaceSource::new(alias.join("new"), alias.join("new"), false).unwrap();
    let authority = fixture
        .authority(
            vec![NativeWorkspaceEntrySpec::new(source, true, false).unwrap()],
            false,
        )
        .unwrap();
    assert!(
        !authority.snapshot().unwrap().entries()[0]
            .source()
            .identity_canonical()
    );
    std::os::unix::fs::symlink(&target, &alias).unwrap();
    std::fs::create_dir(target.join("new")).unwrap();
    let refreshed = authority
        .install(authority.refresh_blocking().unwrap())
        .unwrap();
    assert_eq!(refreshed.entries()[0].source().source(), alias.join("new"));
    assert_eq!(
        refreshed.entries()[0].source().identity(),
        target.join("new")
    );
    assert!(refreshed.entries()[0].source().identity_canonical());
    std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(&fixture.primary, &alias).unwrap();
    assert!(
        authority
            .install(authority.refresh_blocking().unwrap())
            .unwrap()
            .entries()[0]
            .available()
    );
}

#[test]
fn non_unicode_routes_without_loss_and_debug_is_redacted() {
    let fixture = Fixture::new();
    let extra = fixture.directory("extra");
    let authority = fixture
        .authority(vec![spec(&extra, true, false)], false)
        .unwrap();
    let scope = authority.snapshot().unwrap();
    assert_eq!(scope.route(&extra).unwrap().root_identity(), extra);
    let relative = PathBuf::from(OsString::from_vec(vec![b'e', 0xff]));
    assert_eq!(
        scope.route(&extra.join(&relative)).unwrap().relative_path(),
        relative
    );
    assert!(!format!("{scope:?}").contains("machine-god-workspace-authority"));
}

#[test]
fn descriptor_ancestor_detects_real_nested_directories() {
    let fixture = Fixture::new();
    let nested = fixture.directory("primary/nested");
    assert!(descriptor_ancestor(&open(&fixture.primary), &open(&nested)).unwrap());
    assert!(!descriptor_ancestor(&open(&nested), &open(&fixture.primary)).unwrap());
    assert!(!descriptor_ancestor(&open(&fixture.state), &open(&nested)).unwrap());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn terminal_directory_exclusion_tracks_moved_state_and_checks_each_ancestry_step() {
    let fixture = Fixture::new();
    let authority = fixture.authority(vec![], false).unwrap();
    let snapshot = authority.snapshot().unwrap();
    let nested = fixture.directory("state/nested");
    let retained = open(&nested);
    let moved = fixture.primary.join("moved-state");
    std::fs::rename(&fixture.state, &moved).unwrap();
    std::fs::create_dir(&fixture.state).unwrap();
    assert!(
        snapshot.route(&moved.join("nested")).is_ok(),
        "lexical routing alone does not exclude the retained state object"
    );
    assert_eq!(
        snapshot.validate_directory_outside_state(&retained, || Ok(())),
        Err(NativeWorkspaceAuthorityError::OverlappingState)
    );
    snapshot
        .validate_directory_outside_state(&open(&fixture.primary), || Ok(()))
        .unwrap();
    let mut checks = 0;
    assert_eq!(
        snapshot.validate_directory_outside_state(&open(&fixture.primary), || {
            checks += 1;
            if checks == 4 {
                Err(NativeWorkspaceAuthorityError::Unavailable)
            } else {
                Ok(())
            }
        }),
        Err(NativeWorkspaceAuthorityError::Unavailable)
    );
    assert_eq!(
        checks, 4,
        "a cancelled/deadline check stops the native walk"
    );
}

#[test]
fn missing_provisional_leaf_cannot_hide_state_overlap_behind_ancestor_alias() {
    let fixture = Fixture::new();
    let alias = fixture.base.join("alias");
    std::os::unix::fs::symlink(&fixture.state, &alias).unwrap();
    let source =
        NativeWorkspaceSource::new(alias.join("missing"), alias.join("missing"), false).unwrap();
    assert!(matches!(
        fixture.authority(
            vec![NativeWorkspaceEntrySpec::new(source, true, false).unwrap()],
            false
        ),
        Err(NativeWorkspaceAuthorityError::OverlappingState)
    ));
}

#[test]
fn absent_state_retains_exclusion_without_creating_and_allows_unrelated_sibling() {
    let fixture = Fixture::new();
    let future = fixture.state.join("not-created/session");
    let sibling = fixture.directory("state/sibling");
    let authority = NativeWorkspaceAuthority::open_blocking(
        open(&fixture.primary),
        fixture.primary.clone(),
        None,
        future.clone(),
        vec![spec(&sibling, true, false)],
        false,
    )
    .unwrap();
    assert!(!fixture.state.join("not-created").exists());
    assert!(authority.snapshot().unwrap().route(&sibling).is_ok());
    assert!(matches!(
        authority.prepare_blocking(vec![spec(&fixture.state, true, false)], false),
        Err(NativeWorkspaceAuthorityError::OverlappingState)
    ));
    let alias = fixture.base.join("alias");
    std::os::unix::fs::symlink(&fixture.state, &alias).unwrap();
    let source = NativeWorkspaceSource::new(
        alias.join("not-created/session/deeper"),
        alias.join("not-created/session/deeper"),
        false,
    )
    .unwrap();
    assert!(matches!(
        authority.prepare_blocking(
            vec![NativeWorkspaceEntrySpec::new(source, true, false).unwrap()],
            false
        ),
        Err(NativeWorkspaceAuthorityError::OverlappingState)
    ));
    assert!(!fixture.state.join("not-created").exists());
}

#[test]
fn absent_state_exclusion_canonicalizes_ancestor_and_refuses_existing_or_wrong_state() {
    let fixture = Fixture::new();
    let alias = fixture.base.join("alias");
    std::os::unix::fs::symlink(&fixture.state, &alias).unwrap();
    let future = alias.join("future");
    let authority = NativeWorkspaceAuthority::open_blocking(
        open(&fixture.primary),
        fixture.primary.clone(),
        None,
        future,
        vec![],
        false,
    )
    .unwrap();
    assert_eq!(
        authority.snapshot().unwrap().0.state.identity,
        fixture.state.join("future")
    );
    assert!(matches!(
        NativeWorkspaceAuthority::open_blocking(
            open(&fixture.primary),
            fixture.primary.clone(),
            None,
            fixture.state.clone(),
            vec![],
            false
        ),
        Err(NativeWorkspaceAuthorityError::Unavailable)
    ));
    assert!(matches!(
        NativeWorkspaceAuthority::open_blocking(
            open(&fixture.primary),
            fixture.primary.clone(),
            Some(open(&fixture.primary)),
            fixture.state.clone(),
            vec![],
            false
        ),
        Err(NativeWorkspaceAuthorityError::Unavailable)
    ));
    std::fs::create_dir(fixture.state.join("future")).unwrap();
    assert!(matches!(
        authority.refresh_blocking(),
        Err(NativeWorkspaceAuthorityError::Unavailable)
    ));
}

#[test]
fn renamed_exclusion_ancestor_is_not_silently_rebound_on_prepare() {
    let fixture = Fixture::new();
    let authority = NativeWorkspaceAuthority::open_blocking(
        open(&fixture.primary),
        fixture.primary.clone(),
        None,
        fixture.state.join("future"),
        vec![],
        false,
    )
    .unwrap();
    let old = authority.snapshot().unwrap();
    std::fs::rename(&fixture.state, fixture.base.join("moved-state")).unwrap();
    std::fs::create_dir(&fixture.state).unwrap();
    assert!(matches!(
        authority.refresh_blocking(),
        Err(NativeWorkspaceAuthorityError::Unavailable)
    ));
    assert_eq!(old.generation(), authority.snapshot().unwrap().generation());
}

#[test]
fn newly_symlinked_missing_state_prefix_requires_fresh_explicit_authority() {
    let fixture = Fixture::new();
    let authority = NativeWorkspaceAuthority::open_blocking(
        open(&fixture.primary),
        fixture.primary.clone(),
        None,
        fixture.state.join("missing/future"),
        vec![],
        false,
    )
    .unwrap();
    let outside = fixture.directory("outside");
    std::os::unix::fs::symlink(outside, fixture.state.join("missing")).unwrap();
    assert!(matches!(
        authority.refresh_blocking(),
        Err(NativeWorkspaceAuthorityError::Unavailable)
    ));
}
