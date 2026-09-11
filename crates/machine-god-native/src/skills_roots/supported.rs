use super::super::*;
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "mg-skill-roots-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        Self(fs::canonicalize(directory).unwrap())
    }
    fn authority(&self, relative: &str) -> NativeSkillDirectoryAuthority {
        let path = self.0.join(relative);
        fs::create_dir_all(&path).unwrap();
        NativeSkillDirectoryAuthority::from_directory(Arc::new(File::open(&path).unwrap()), path)
            .unwrap()
    }
    fn skill(&self, relative: &str, name: &str) {
        let path = self.0.join(relative);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("SKILL.md"),
            format!("---\nname: {name}\n---\noriginal body"),
        )
        .unwrap();
    }
    fn managed(&self) -> (Arc<File>, NativeSkillRoot) {
        let authority = self.authority("state");
        let root = NativeSkillRoot::from_directory(
            authority.directory.clone(),
            "skills".into(),
            authority.path.clone(),
            authority.path.join("skills"),
            NativeSkillSource::Managed,
            NativeSkillLinkPolicy::Reject,
        )
        .unwrap();
        (authority.directory, root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn order_is_nearest_workspace_then_managed_then_pinned_home_roots() {
    let fixture = Fixture::new();
    let workspace = fixture.authority("home/project/app");
    let home = fixture.authority("home");
    let (_, managed) = fixture.managed();
    let workspace_paths = [
        "skills",
        ".opencode/skills",
        ".codex/skills",
        ".claude/skills",
        ".agents/skills",
        ".claw/skills",
    ];
    let home_paths = [
        ".fx/skills",
        ".config/opencode/skills",
        ".codex/skills",
        ".claude/skills",
        ".agents/skills",
        ".claw/skills",
    ];
    for level in ["home/project/app", "home/project"] {
        for path in workspace_paths {
            fixture.skill(&format!("{level}/{path}/entry"), "duplicate");
        }
    }
    for path in home_paths {
        fixture.skill(&format!("home/{path}/entry"), "duplicate");
    }
    fixture.skill("home/skills/ignored", "must-not-scan-home-workspace-root");
    fixture.skill("state/skills/entry", "duplicate");
    let catalog = compose_native_skill_catalog(
        Some(&workspace),
        Some(&home),
        Some(managed),
        &CancellationToken::new(),
    )
    .unwrap();
    let snapshot = catalog.discover(&CancellationToken::new()).unwrap();
    assert!(snapshot.complete());
    assert_eq!(snapshot.entries().len(), 19);
    let expected_workspace = [
        NativeSkillSource::WorkspaceShared,
        NativeSkillSource::WorkspaceOpencode,
        NativeSkillSource::WorkspaceCodex,
        NativeSkillSource::WorkspaceClaude,
        NativeSkillSource::WorkspaceAgents,
        NativeSkillSource::WorkspaceClaw,
    ];
    let expected = expected_workspace
        .into_iter()
        .chain(expected_workspace)
        .chain([
            NativeSkillSource::Managed,
            NativeSkillSource::GlobalFx,
            NativeSkillSource::GlobalOpencode,
            NativeSkillSource::GlobalCodex,
            NativeSkillSource::GlobalClaude,
            NativeSkillSource::GlobalAgents,
            NativeSkillSource::GlobalClaw,
        ])
        .collect::<Vec<_>>();
    assert_eq!(
        snapshot
            .entries()
            .iter()
            .map(crate::skills_catalog::NativeSkillEntry::source)
            .collect::<Vec<_>>(),
        expected
    );
    assert!(
        snapshot.entries()[0]
            .location()
            .starts_with(&workspace.path)
    );
    assert!(
        snapshot.entries()[6]
            .location()
            .starts_with(fixture.0.join("home/project/skills"))
    );
    assert_eq!(
        snapshot.resolve("duplicate", None).unwrap_err(),
        NativeSkillCatalogError::AmbiguousName
    );
}

#[test]
fn missing_children_are_empty_and_composition_creates_nothing() {
    let fixture = Fixture::new();
    let workspace = fixture.authority("home/workspace");
    let home = fixture.authority("home");
    let (_, managed) = fixture.managed();
    let catalog = compose_native_skill_catalog(
        Some(&workspace),
        Some(&home),
        Some(managed),
        &CancellationToken::new(),
    )
    .unwrap();
    let snapshot = catalog.discover(&CancellationToken::new()).unwrap();
    assert!(snapshot.complete());
    assert!(snapshot.entries().is_empty());
    assert_eq!(fs::read_dir(&workspace.path).unwrap().count(), 0);
    assert_eq!(fs::read_dir(fixture.0.join("state")).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&home.path).unwrap().count(), 1);
    let empty = compose_native_skill_catalog(None, None, None, &CancellationToken::new()).unwrap();
    assert!(
        empty
            .discover(&CancellationToken::new())
            .unwrap()
            .complete()
    );
}

#[test]
fn home_identity_stops_ancestry_even_with_legitimate_alias_label() {
    let fixture = Fixture::new();
    let workspace = fixture.authority("home/project");
    fixture.skill("home/skills/not-workspace", "excluded");
    fixture.skill("home/.agents/skills/global", "included");
    fs::create_dir(fixture.0.join("labels")).unwrap();
    std::os::unix::fs::symlink("../home", fixture.0.join("labels/alias")).unwrap();
    let home = NativeSkillDirectoryAuthority::from_directory(
        Arc::new(File::open(fixture.0.join("labels/alias")).unwrap()),
        fixture.0.join("labels/alias"),
    )
    .unwrap();
    let catalog = compose_native_skill_catalog(
        Some(&workspace),
        Some(&home),
        None,
        &CancellationToken::new(),
    )
    .unwrap();
    let snapshot = catalog.discover(&CancellationToken::new()).unwrap();
    assert_eq!(snapshot.entries().len(), 1);
    assert_eq!(snapshot.entries()[0].metadata.name, "included");
    assert_eq!(
        snapshot.entries()[0].source(),
        NativeSkillSource::GlobalAgents
    );
    assert!(snapshot.entries()[0].location().starts_with(home.path()));
}

#[test]
fn workspace_equal_home_has_only_global_roots() {
    let fixture = Fixture::new();
    let home = fixture.authority("home");
    fixture.skill("home/skills/excluded", "excluded");
    fixture.skill("home/.fx/skills/global", "included");
    let catalog =
        compose_native_skill_catalog(Some(&home), Some(&home), None, &CancellationToken::new())
            .unwrap();
    let snapshot = catalog.discover(&CancellationToken::new()).unwrap();
    assert_eq!(snapshot.entries().len(), 1);
    assert_eq!(snapshot.entries()[0].source(), NativeSkillSource::GlobalFx);
}

#[test]
fn managed_root_preserves_exact_directory_arc_not_just_inode_or_label() {
    let fixture = Fixture::new();
    fixture.skill("state/skills/entry", "managed");
    let (directory, managed) = fixture.managed();
    let catalog =
        compose_native_skill_catalog(None, None, Some(managed), &CancellationToken::new()).unwrap();
    let snapshot = catalog.discover(&CancellationToken::new()).unwrap();
    let selection = snapshot.entries()[0].selection_ref();
    assert!(selection.belongs_to_managed_directory(&directory));
    let independent = Arc::new(File::open(fixture.0.join("state")).unwrap());
    assert!(!selection.belongs_to_managed_directory(&independent));
    fs::rename(fixture.0.join("state"), fixture.0.join("old-state")).unwrap();
    fixture.skill("state/skills/entry", "replacement");
    assert_eq!(
        catalog
            .materialize(selection, &CancellationToken::new())
            .unwrap()
            .metadata
            .name,
        "managed"
    );
}

#[test]
fn compatibility_aliases_deduplicate_by_identity_without_hiding_duplicate_names() {
    let fixture = Fixture::new();
    let home = fixture.authority("home");
    fixture.skill("home/.agents/skills/one", "same");
    fixture.skill("home/.agents/skills/two", "same");
    fs::create_dir(fixture.0.join("home/.claude")).unwrap();
    std::os::unix::fs::symlink("../.agents/skills", fixture.0.join("home/.claude/skills")).unwrap();
    let catalog =
        compose_native_skill_catalog(None, Some(&home), None, &CancellationToken::new()).unwrap();
    let snapshot = catalog.discover(&CancellationToken::new()).unwrap();
    assert_eq!(snapshot.entries().len(), 2);
    assert!(
        snapshot
            .entries()
            .iter()
            .all(|entry| entry.source() == NativeSkillSource::GlobalClaude)
    );
    assert_eq!(
        snapshot.resolve("same", None).unwrap_err(),
        NativeSkillCatalogError::AmbiguousName
    );
}

#[test]
fn moved_workspace_or_parent_before_composition_cannot_redirect_ancestry() {
    for parent in [false, true] {
        let fixture = Fixture::new();
        let workspace = fixture.authority("home/project/workspace");
        let home = fixture.authority("home");
        let old = if parent {
            "home/project"
        } else {
            "home/project/workspace"
        };
        fs::rename(fixture.0.join(old), fixture.0.join("moved")).unwrap();
        fs::create_dir_all(fixture.0.join(old)).unwrap();
        assert_eq!(
            compose_native_skill_catalog(
                Some(&workspace),
                Some(&home),
                None,
                &CancellationToken::new()
            )
            .unwrap_err(),
            NativeSkillRootsError::ChangedAncestry
        );
    }
}

#[test]
fn replaced_home_identity_is_a_stop_not_permission_to_climb_above_it() {
    let fixture = Fixture::new();
    let home = fixture.authority("home");
    fs::rename(fixture.0.join("home"), fixture.0.join("old-home")).unwrap();
    let workspace = fixture.authority("home/workspace");
    assert_eq!(
        compose_native_skill_catalog(
            Some(&workspace),
            Some(&home),
            None,
            &CancellationToken::new()
        )
        .unwrap_err(),
        NativeSkillRootsError::ChangedAncestry
    );
}

#[test]
fn composed_roots_survive_replacement_of_all_captured_labels() {
    let fixture = Fixture::new();
    let home = fixture.authority("home");
    let workspace = fixture.authority("home/project/workspace");
    fixture.skill("home/project/workspace/skills/local", "local");
    fixture.skill("home/project/skills/ancestor", "ancestor");
    fixture.skill("home/.fx/skills/global", "global");
    let catalog = compose_native_skill_catalog(
        Some(&workspace),
        Some(&home),
        None,
        &CancellationToken::new(),
    )
    .unwrap();
    fs::rename(fixture.0.join("home"), fixture.0.join("old-home")).unwrap();
    fixture.skill("home/project/workspace/skills/local", "replacement");
    let snapshot = catalog.discover(&CancellationToken::new()).unwrap();
    assert_eq!(
        snapshot
            .entries()
            .iter()
            .map(|entry| entry.metadata.name.as_str())
            .collect::<Vec<_>>(),
        ["local", "ancestor", "global"]
    );
    for entry in snapshot.entries() {
        assert!(
            catalog
                .materialize(entry.selection_ref(), &CancellationToken::new())
                .unwrap()
                .text
                .contains("original body")
        );
    }
}

#[test]
fn exact_ancestor_bound_is_accepted_and_excess_fails_closed() {
    let fixture = Fixture::new();
    let home = fixture.authority("home");
    for levels in [20, 21] {
        let relative = format!("home/{}", vec!["d"; levels].join("/"));
        let workspace = fixture.authority(&relative);
        let result = compose_native_skill_catalog(
            Some(&workspace),
            Some(&home),
            None,
            &CancellationToken::new(),
        );
        if levels == 20 {
            assert!(result.is_ok());
        } else {
            assert_eq!(result.unwrap_err(), NativeSkillRootsError::ResourceLimit);
        }
    }
}

#[test]
fn invalid_managed_provenance_is_rejected_before_descriptor_inspection() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("file"), "not a directory").unwrap();
    let file = Arc::new(File::open(fixture.0.join("file")).unwrap());
    let authority =
        NativeSkillDirectoryAuthority::from_directory(file.clone(), fixture.0.join("file"))
            .unwrap();
    let foreign = NativeSkillRoot::from_directory(
        file,
        "skills".into(),
        fixture.0.clone(),
        fixture.0.join("skills"),
        NativeSkillSource::GlobalFx,
        NativeSkillLinkPolicy::Reject,
    )
    .unwrap();
    assert_eq!(
        compose_native_skill_catalog(
            Some(&authority),
            None,
            Some(foreign),
            &CancellationToken::new()
        )
        .unwrap_err(),
        NativeSkillRootsError::InvalidAuthority
    );
    assert_eq!(
        compose_native_skill_catalog(Some(&authority), None, None, &CancellationToken::new())
            .unwrap_err(),
        NativeSkillRootsError::InvalidAuthority
    );
}

#[test]
fn authority_construction_is_inert_bounded_and_redacted() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("file"), "body").unwrap();
    let file = Arc::new(File::open(fixture.0.join("file")).unwrap());
    let authority =
        NativeSkillDirectoryAuthority::from_directory(file.clone(), "/not/existing/PRIVATE".into())
            .unwrap();
    assert!(!format!("{authority:?}").contains("PRIVATE"));
    for path in [
        "relative".into(),
        "/parent/../child".into(),
        "/bad\\name".into(),
        PathBuf::from(format!("/{}", "x".repeat(4096))),
    ] {
        assert_eq!(
            NativeSkillDirectoryAuthority::from_directory(file.clone(), path).unwrap_err(),
            NativeSkillRootsError::InvalidAuthority
        );
    }
}

#[test]
fn oversized_owned_authority_label_is_rejected_without_copying_it() {
    let fixture = Fixture::new();
    let directory = Arc::new(File::open(&fixture.0).unwrap());
    // The caller owns these input allocations; measure only adapter work.
    let small = PathBuf::from(format!("/{}", "x".repeat(4096)));
    let huge = PathBuf::from(format!("/{}", "x".repeat(8 * 1024 * 1024)));
    allocation_counter::measure(|| {});
    let measure = |path| {
        allocation_counter::measure(|| {
            assert_eq!(
                NativeSkillDirectoryAuthority::from_directory(directory.clone(), path).unwrap_err(),
                NativeSkillRootsError::InvalidAuthority
            );
        })
    };
    let small_allocations = measure(small);
    let huge_allocations = measure(huge);
    assert_eq!(huge_allocations.bytes_total, small_allocations.bytes_total);
    assert_eq!(huge_allocations.bytes_total, 0, "{huge_allocations:?}");
}

#[test]
fn authority_label_byte_and_component_bounds_are_inclusive() {
    use crate::skills_catalog::{MAX_NATIVE_SKILL_PATH_BYTES, MAX_NATIVE_SKILL_PATH_COMPONENTS};
    let fixture = Fixture::new();
    let directory = Arc::new(File::open(&fixture.0).unwrap());
    // Construction is lexical: even a nonexistent long component needs no I/O.
    let longest = PathBuf::from(format!("/{}", "x".repeat(MAX_NATIVE_SKILL_PATH_BYTES - 1)));
    let authority =
        NativeSkillDirectoryAuthority::from_directory(directory.clone(), longest.clone()).unwrap();
    assert_eq!(authority.path(), longest);
    let deepest = PathBuf::from(format!(
        "/{}",
        vec!["x"; MAX_NATIVE_SKILL_PATH_COMPONENTS - 1].join("/")
    ));
    assert_eq!(
        deepest.components().count(),
        MAX_NATIVE_SKILL_PATH_COMPONENTS
    );
    assert!(
        NativeSkillDirectoryAuthority::from_directory(directory.clone(), deepest.clone()).is_ok()
    );
    for path in [
        PathBuf::from(format!("{}x", longest.display())),
        deepest.join("x"),
    ] {
        assert_eq!(
            NativeSkillDirectoryAuthority::from_directory(directory.clone(), path).unwrap_err(),
            NativeSkillRootsError::InvalidAuthority
        );
    }
}

#[test]
fn authority_preflight_preserves_normalization_and_rejects_oversized_raw_spelling() {
    let fixture = Fixture::new();
    let directory = Arc::new(File::open(&fixture.0).unwrap());
    for path in ["/α//./β/", "/./α/β", "/α/β"] {
        let authority =
            NativeSkillDirectoryAuthority::from_directory(directory.clone(), path.into()).unwrap();
        assert_eq!(authority.path(), Path::new("/α/β"));
    }
    // A short normalized result does not make an oversized raw label valid.
    let oversized = PathBuf::from(format!("/α/{}β", "/".repeat(4096)));
    assert_eq!(
        NativeSkillDirectoryAuthority::from_directory(directory, oversized).unwrap_err(),
        NativeSkillRootsError::InvalidAuthority
    );
}

#[test]
fn authority_preflight_keeps_invalid_utf8_errors_below_and_above_byte_limit() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let fixture = Fixture::new();
    let directory = Arc::new(File::open(&fixture.0).unwrap());
    for length in [2, crate::skills_catalog::MAX_NATIVE_SKILL_PATH_BYTES + 1] {
        let mut bytes = vec![b'x'; length];
        bytes[0] = b'/';
        bytes[length - 1] = 0xff;
        let path = PathBuf::from(OsString::from_vec(bytes));
        assert_eq!(
            NativeSkillDirectoryAuthority::from_directory(directory.clone(), path).unwrap_err(),
            NativeSkillRootsError::InvalidAuthority
        );
    }
}

#[test]
fn absent_or_nonancestor_home_allows_bounded_climb_to_filesystem_root() {
    let fixture = Fixture::new();
    let workspace = fixture.authority("outside/workspace");
    let home = fixture.authority("home");
    // Composition only opens parents, never scans unrelated host children.
    assert!(
        compose_native_skill_catalog(Some(&workspace), None, None, &CancellationToken::new())
            .is_ok()
    );
    assert!(
        compose_native_skill_catalog(
            Some(&workspace),
            Some(&home),
            None,
            &CancellationToken::new()
        )
        .is_ok()
    );
    let filesystem_root = NativeSkillDirectoryAuthority::from_directory(
        Arc::new(File::open("/").unwrap()),
        "/".into(),
    )
    .unwrap();
    assert!(
        compose_native_skill_catalog(
            Some(&filesystem_root),
            None,
            None,
            &CancellationToken::new()
        )
        .is_ok()
    );
}

#[test]
fn cancellation_precedes_even_invalid_descriptor_observation() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("file"), "body").unwrap();
    let authority = NativeSkillDirectoryAuthority::from_directory(
        Arc::new(File::open(fixture.0.join("file")).unwrap()),
        fixture.0.join("file"),
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        compose_native_skill_catalog(Some(&authority), Some(&authority), None, &cancellation)
            .unwrap_err(),
        NativeSkillRootsError::Cancelled
    );
}
