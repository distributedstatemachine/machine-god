use super::super::*;
use machine_god_core::CancellationToken;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "mg-skills-catalog-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        Self(directory)
    }
    fn write(&self, relative: &str, bytes: impl AsRef<[u8]>) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    fn root(
        &self,
        relative: &str,
        source: NativeSkillSource,
        links: NativeSkillLinkPolicy,
    ) -> NativeSkillRoot {
        NativeSkillRoot::from_directory(
            Arc::new(fs::File::open(&self.0).unwrap()),
            relative.into(),
            self.0.clone(),
            self.0.join(relative),
            source,
            links,
        )
        .unwrap()
    }
    fn catalog(&self) -> NativeSkillCatalog {
        NativeSkillCatalog::new(vec![self.root(
            "skills",
            NativeSkillSource::WorkspaceShared,
            NativeSkillLinkPolicy::Contained,
        )])
        .unwrap()
    }
    fn snapshot(&self) -> NativeSkillSnapshot {
        self.catalog().discover(&CancellationToken::new()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn cancelled_empty_catalog_is_inert() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let catalog = NativeSkillCatalog::new(Vec::new()).unwrap();
    assert_eq!(
        catalog.discover(&cancellation).unwrap_err(),
        NativeSkillCatalogError::Cancelled
    );
}

#[test]
fn discovery_is_one_level_ordered_and_skips_managed_transactions() {
    let fixture = Fixture::new();
    fixture.write("skills/zeta/SKILL.md", "zeta body");
    fixture.write(
        "skills/alpha/SKILL.md",
        "---\nname: advertised\ndescription: desc\n---\nbody",
    );
    fixture.write("skills/.staging/temporary/SKILL.md", "hidden");
    fixture.write("skills/nested/child/SKILL.md", "not discovered");
    fixture.write("skills/not-directory", "ignored");
    let snapshot = fixture.snapshot();
    assert!(snapshot.complete());
    assert!(snapshot.diagnostics().is_empty());
    assert_eq!(
        snapshot
            .entries()
            .iter()
            .map(|entry| entry.metadata.name.as_str())
            .collect::<Vec<_>>(),
        ["advertised", "zeta"]
    );
}

#[test]
fn root_order_and_same_directory_alias_deduplication_are_stable() {
    let fixture = Fixture::new();
    fixture.write("skills/first/SKILL.md", "body");
    fixture.write("other/second/SKILL.md", "body");
    std::os::unix::fs::symlink("skills", fixture.0.join("alias")).unwrap();
    let roots = ["skills", "alias", "other"].map(|path| {
        fixture.root(
            path,
            NativeSkillSource::WorkspaceShared,
            NativeSkillLinkPolicy::Contained,
        )
    });
    let snapshot = NativeSkillCatalog::new(roots.into())
        .unwrap()
        .discover(&CancellationToken::new())
        .unwrap();
    assert_eq!(snapshot.entries().len(), 2);
    assert_eq!(
        snapshot.entries()[0].location(),
        fixture.0.join("skills/first")
    );
    assert_eq!(snapshot.entries()[1].metadata.name, "second");
}

#[test]
fn duplicate_names_require_exact_location() {
    let fixture = Fixture::new();
    for name in ["alpha", "beta"] {
        fixture.write(
            &format!("skills/{name}/SKILL.md"),
            "---\nname: duplicate\n---\nbody",
        );
    }
    let snapshot = fixture.snapshot();
    assert_eq!(
        snapshot.resolve("duplicate", None).unwrap_err(),
        NativeSkillCatalogError::AmbiguousName
    );
    let location = fixture.0.join("skills/beta");
    assert_eq!(
        snapshot
            .resolve("duplicate", Some(&location))
            .unwrap()
            .location(),
        location
    );
    assert_eq!(
        snapshot.resolve("other", Some(&location)).unwrap_err(),
        NativeSkillCatalogError::NameLocationMismatch
    );
    assert_eq!(
        snapshot
            .resolve("duplicate", Some(Path::new("/not-advertised")))
            .unwrap_err(),
        NativeSkillCatalogError::NotFound
    );
}

#[test]
fn malformed_neighbor_makes_name_only_resolution_incomplete() {
    let fixture = Fixture::new();
    fixture.write("skills/valid/SKILL.md", "body");
    fixture.write("skills/bad/SKILL.md", "---\ndescription: no name\n---");
    let snapshot = fixture.snapshot();
    assert!(!snapshot.complete());
    assert_eq!(snapshot.entries().len(), 1);
    assert_eq!(snapshot.diagnostics().len(), 1);
    assert_eq!(
        snapshot.resolve("valid", None).unwrap_err(),
        NativeSkillCatalogError::IncompleteDiscovery
    );
    assert!(
        snapshot
            .resolve("valid", Some(&fixture.0.join("skills/valid")))
            .is_ok()
    );
}

#[test]
fn missing_root_is_empty_and_invalid_root_is_diagnosed() {
    let fixture = Fixture::new();
    assert!(fixture.snapshot().complete());
    fixture.write("skills", "not a directory");
    let snapshot = fixture.snapshot();
    assert!(!snapshot.complete());
    assert!(snapshot.diagnostics()[0].root);
}

#[test]
fn broken_symlink_roots_and_candidates_are_not_reported_as_complete_empty_roots() {
    let fixture = Fixture::new();
    std::os::unix::fs::symlink("missing-target", fixture.0.join("skills")).unwrap();
    assert!(!fixture.snapshot().complete());
    fs::remove_file(fixture.0.join("skills")).unwrap();
    fs::create_dir(fixture.0.join("skills")).unwrap();
    std::os::unix::fs::symlink("missing-target", fixture.0.join("skills/broken")).unwrap();
    let snapshot = fixture.snapshot();
    assert!(!snapshot.complete());
    assert_eq!(
        snapshot.diagnostics()[0].cause,
        NativeSkillCatalogError::PathRejected
    );
}

#[test]
fn exact_materialization_preserves_bytes_and_never_rescans() {
    let fixture = Fixture::new();
    let text = "---\nname: selected\n---\n\n# Exact bytes\n";
    fixture.write("skills/location/SKILL.md", text);
    let catalog = fixture.catalog();
    let snapshot = catalog.discover(&CancellationToken::new()).unwrap();
    let selection = snapshot.resolve("selected", None).unwrap();
    fixture.write("skills/new-duplicate/SKILL.md", text);
    let output = catalog
        .materialize(&selection, &CancellationToken::new())
        .unwrap();
    assert_eq!(output.text, text);
    assert_eq!(output.selection, selection);
    assert!(
        selection.retained_bytes()
            >= selection.name().len() + selection.location().as_os_str().len()
    );
    assert_eq!(
        catalog
            .clone()
            .materialize(&selection, &CancellationToken::new())
            .unwrap()
            .text,
        text
    );
}

#[test]
fn foreign_catalog_selection_is_not_an_authority() {
    let fixture = Fixture::new();
    fixture.write("skills/selected/SKILL.md", "body");
    let selection = fixture.snapshot().resolve("selected", None).unwrap();
    assert_eq!(
        fixture
            .catalog()
            .materialize(&selection, &CancellationToken::new())
            .unwrap_err(),
        NativeSkillCatalogError::WrongAuthority
    );
}

#[test]
fn replacing_selected_file_or_directory_rejects_old_revision() {
    let fixture = Fixture::new();
    fixture.write("skills/selected/SKILL.md", "original");
    let catalog = fixture.catalog();
    let selection = catalog
        .discover(&CancellationToken::new())
        .unwrap()
        .resolve("selected", None)
        .unwrap();
    fixture.write("skills/selected/replacement", "different");
    fs::rename(
        fixture.0.join("skills/selected/replacement"),
        fixture.0.join("skills/selected/SKILL.md"),
    )
    .unwrap();
    assert_eq!(
        catalog
            .materialize(&selection, &CancellationToken::new())
            .unwrap_err(),
        NativeSkillCatalogError::StaleSelection
    );
    let selected = catalog
        .discover(&CancellationToken::new())
        .unwrap()
        .resolve("selected", None)
        .unwrap();
    fs::rename(fixture.0.join("skills/selected"), fixture.0.join("retired")).unwrap();
    fixture.write("skills/selected/SKILL.md", "different");
    assert_eq!(
        catalog
            .materialize(&selected, &CancellationToken::new())
            .unwrap_err(),
        NativeSkillCatalogError::StaleSelection
    );
}

#[test]
fn in_place_body_change_and_prefix_digest_change_are_stale() {
    let fixture = Fixture::new();
    fixture.write(
        "skills/selected/SKILL.md",
        "---\nname: selected\n---\nold body",
    );
    let catalog = fixture.catalog();
    let mut selection = catalog
        .discover(&CancellationToken::new())
        .unwrap()
        .resolve("selected", None)
        .unwrap();
    selection.prefix_digest[0] ^= 1;
    assert_eq!(
        catalog
            .materialize(&selection, &CancellationToken::new())
            .unwrap_err(),
        NativeSkillCatalogError::StaleSelection
    );
    let selection = catalog
        .discover(&CancellationToken::new())
        .unwrap()
        .resolve("selected", None)
        .unwrap();
    fixture.write(
        "skills/selected/SKILL.md",
        "---\nname: selected\n---\nnew body",
    );
    assert_eq!(
        catalog
            .materialize(&selection, &CancellationToken::new())
            .unwrap_err(),
        NativeSkillCatalogError::StaleSelection
    );
}

#[test]
fn retained_base_survives_namespace_rename_without_redirecting() {
    let fixture = Fixture::new();
    fixture.write("base/skills/selected/SKILL.md", "retained");
    let base = fixture.0.join("base");
    let root = NativeSkillRoot::from_directory(
        Arc::new(fs::File::open(&base).unwrap()),
        "skills".into(),
        base.clone(),
        base.join("skills"),
        NativeSkillSource::WorkspaceShared,
        NativeSkillLinkPolicy::Contained,
    )
    .unwrap();
    let catalog = NativeSkillCatalog::new(vec![root]).unwrap();
    let selection = catalog
        .discover(&CancellationToken::new())
        .unwrap()
        .resolve("selected", None)
        .unwrap();
    fs::rename(&base, fixture.0.join("retained-base")).unwrap();
    fixture.write("base/skills/selected/SKILL.md", "replacement");
    assert_eq!(
        catalog
            .materialize(&selection, &CancellationToken::new())
            .unwrap()
            .text,
        "retained"
    );
}

#[test]
fn contained_relative_and_absolute_links_work_but_retargeting_is_stale() {
    let fixture = Fixture::new();
    fixture.write("real/target/SKILL.md", "---\nname: selected\n---\nbody");
    fs::create_dir(fixture.0.join("skills")).unwrap();
    let alias = fixture.0.join("skills/alias");
    std::os::unix::fs::symlink("../real/target", &alias).unwrap();
    let catalog = fixture.catalog();
    let selection = catalog
        .discover(&CancellationToken::new())
        .unwrap()
        .resolve("selected", None)
        .unwrap();
    assert!(
        catalog
            .materialize(&selection, &CancellationToken::new())
            .is_ok()
    );
    fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(fixture.0.join("real/target"), &alias).unwrap();
    assert!(
        catalog
            .materialize(&selection, &CancellationToken::new())
            .is_ok()
    );
    fixture.write("real/other/SKILL.md", "---\nname: selected\n---\nbody");
    fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink("../real/other", &alias).unwrap();
    assert_eq!(
        catalog
            .materialize(&selection, &CancellationToken::new())
            .unwrap_err(),
        NativeSkillCatalogError::StaleSelection
    );
}

#[test]
fn escaping_links_and_cycles_are_bounded_and_diagnosed() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("skills")).unwrap();
    std::os::unix::fs::symlink("../../outside", fixture.0.join("skills/relative")).unwrap();
    std::os::unix::fs::symlink("/outside", fixture.0.join("skills/absolute")).unwrap();
    std::os::unix::fs::symlink("cycle", fixture.0.join("skills/cycle")).unwrap();
    let snapshot = fixture.snapshot();
    assert!(!snapshot.complete());
    assert!(snapshot.entries().is_empty());
    assert!(
        snapshot
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.cause == NativeSkillCatalogError::PathRejected)
    );
    assert!(
        snapshot
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.cause == NativeSkillCatalogError::ResourceLimit)
    );
}

#[test]
fn managed_directory_links_and_all_final_file_links_are_rejected() {
    let fixture = Fixture::new();
    fixture.write("real/selected/SKILL.md", "body");
    fs::create_dir(fixture.0.join("skills")).unwrap();
    std::os::unix::fs::symlink("../real/selected", fixture.0.join("skills/alias")).unwrap();
    let root = fixture.root(
        "skills",
        NativeSkillSource::Managed,
        NativeSkillLinkPolicy::Reject,
    );
    let snapshot = NativeSkillCatalog::new(vec![root])
        .unwrap()
        .discover(&CancellationToken::new())
        .unwrap();
    assert!(!snapshot.complete());
    assert!(snapshot.entries().is_empty());
    fs::remove_file(fixture.0.join("skills/alias")).unwrap();
    fs::create_dir(fixture.0.join("skills/alias")).unwrap();
    std::os::unix::fs::symlink(
        "../../real/selected/SKILL.md",
        fixture.0.join("skills/alias/SKILL.md"),
    )
    .unwrap();
    assert!(!fixture.snapshot().complete());
    assert!(fixture.snapshot().entries().is_empty());
}

#[test]
fn nonregular_metadata_and_invalid_utf8_are_not_read_as_instructions() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.0.join("skills/directory/SKILL.md")).unwrap();
    assert!(!fixture.snapshot().complete());
    fixture.write("skills/invalid/SKILL.md", b"body\xff");
    let catalog = fixture.catalog();
    let snapshot = catalog.discover(&CancellationToken::new()).unwrap();
    let selected = snapshot
        .resolve("invalid", Some(&fixture.0.join("skills/invalid")))
        .unwrap();
    assert_eq!(
        catalog
            .materialize(&selected, &CancellationToken::new())
            .unwrap_err(),
        NativeSkillCatalogError::InvalidUtf8
    );
}

#[test]
fn oversized_body_is_discoverable_but_not_materializable() {
    let fixture = Fixture::new();
    let mut body = b"---\nname: large\n---\n".to_vec();
    body.resize(MAX_NATIVE_SKILL_MATERIALIZED_BYTES + 1, b'x');
    fixture.write("skills/large/SKILL.md", body);
    let catalog = fixture.catalog();
    let snapshot = catalog.discover(&CancellationToken::new()).unwrap();
    assert!(snapshot.complete());
    let selected = snapshot.resolve("large", None).unwrap();
    assert_eq!(
        catalog
            .materialize(&selected, &CancellationToken::new())
            .unwrap_err(),
        NativeSkillCatalogError::ResourceLimit
    );
}

#[test]
fn query_is_bounded_case_insensitive_and_stable() {
    let fixture = Fixture::new();
    fixture.write(
        "skills/alpha/SKILL.md",
        "---\nname: Alpha\ndescription: Regression checks\n---\n",
    );
    fixture.write("skills/beta/SKILL.md", "body");
    let snapshot = fixture.snapshot();
    assert_eq!(
        snapshot.query("REGRESSION", 10).unwrap()[0].metadata.name,
        "Alpha"
    );
    assert_eq!(snapshot.query("", 1).unwrap().len(), 1);
    assert!(snapshot.query("", 0).unwrap().is_empty());
    assert!(
        snapshot
            .query(&"x".repeat(MAX_NATIVE_SKILL_QUERY_BYTES + 1), 1)
            .is_err()
    );
    assert!(snapshot.query("", MAX_NATIVE_SKILL_QUERY_ROWS + 1).is_err());
}

#[test]
fn unchanged_observations_have_stable_generation_and_changes_do_not() {
    let fixture = Fixture::new();
    fixture.write("skills/one/SKILL.md", "body");
    let catalog = fixture.catalog();
    let first = catalog.discover(&CancellationToken::new()).unwrap();
    let second = catalog.discover(&CancellationToken::new()).unwrap();
    assert_eq!(first.generation(), second.generation());
    assert_eq!(
        first.entries()[0].selection(),
        second.entries()[0].selection()
    );
    fixture.write("skills/two/SKILL.md", "body");
    assert_ne!(
        first.generation(),
        catalog
            .discover(&CancellationToken::new())
            .unwrap()
            .generation()
    );
}

#[test]
fn candidate_bound_marks_incomplete_instead_of_unique() {
    let fixture = Fixture::new();
    for index in 0..=MAX_NATIVE_SKILL_CANDIDATES {
        fs::create_dir_all(fixture.0.join(format!("skills/item-{index:04}"))).unwrap();
    }
    let snapshot = fixture.snapshot();
    assert!(!snapshot.complete());
    assert!(
        snapshot
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.cause == NativeSkillCatalogError::ResourceLimit)
    );
}

#[test]
fn constructor_paths_and_root_count_are_bounded_without_io() {
    let fixture = Fixture::new();
    let descriptor = Arc::new(fs::File::open(&fixture.0).unwrap());
    let construct = |relative: &str, links| {
        NativeSkillRoot::from_directory(
            Arc::clone(&descriptor),
            relative.into(),
            fixture.0.clone(),
            fixture.0.join(relative),
            NativeSkillSource::Managed,
            links,
        )
    };
    assert!(construct("absent", NativeSkillLinkPolicy::Reject).is_ok());
    assert!(construct("../outside", NativeSkillLinkPolicy::Reject).is_err());
    assert!(construct("skills", NativeSkillLinkPolicy::Contained).is_err());
    let root = construct("absent", NativeSkillLinkPolicy::Reject).unwrap();
    assert_eq!(
        NativeSkillCatalog::new(vec![root; MAX_NATIVE_SKILL_ROOTS + 1]).unwrap_err(),
        NativeSkillCatalogError::ResourceLimit
    );
}

#[test]
fn cancellation_precedes_materialization_and_debug_redacts_text() {
    let fixture = Fixture::new();
    fixture.write("skills/private-name/SKILL.md", "private body");
    let catalog = fixture.catalog();
    let snapshot = catalog.discover(&CancellationToken::new()).unwrap();
    let selected = snapshot.resolve("private-name", None).unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        catalog.materialize(&selected, &cancellation).unwrap_err(),
        NativeSkillCatalogError::Cancelled
    );
    for debug in [
        format!("{catalog:?}"),
        format!("{snapshot:?}"),
        format!("{selected:?}"),
        format!(
            "{:?}",
            catalog
                .materialize(&selected, &CancellationToken::new())
                .unwrap()
        ),
    ] {
        assert!(!debug.contains("private-name"));
        assert!(!debug.contains("private body"));
    }
}

#[test]
fn malformed_or_cyclic_candidates_do_not_hide_valid_neighbors() {
    let fixture = Fixture::new();
    let mut header = b"---\nname: too-large\n#".to_vec();
    header.resize(
        crate::skills_metadata::MAX_NATIVE_SKILL_HEADER_BYTES + 1,
        b'x',
    );
    fixture.write("skills/alpha/SKILL.md", header);
    std::os::unix::fs::symlink("cycle", fixture.0.join("skills/cycle")).unwrap();
    fixture.write("skills/zeta/SKILL.md", "valid neighbor");
    let snapshot = fixture.snapshot();
    assert!(!snapshot.complete());
    assert_eq!(snapshot.entries().len(), 1);
    assert_eq!(snapshot.entries()[0].metadata.name, "zeta");
}

#[test]
fn diagnostics_have_an_independent_retention_bound() {
    let fixture = Fixture::new();
    for index in 0..MAX_NATIVE_SKILL_DIAGNOSTICS + 10 {
        fixture.write(&format!("skills/invalid-{index:04}/SKILL.md"), "---\n---\n");
    }
    let snapshot = fixture.snapshot();
    assert!(!snapshot.complete());
    assert_eq!(snapshot.diagnostics().len(), MAX_NATIVE_SKILL_DIAGNOSTICS);
}

#[test]
fn aggregate_metadata_reads_are_bounded_before_more_candidates() {
    let fixture = Fixture::new();
    let text = vec![b'x'; 16 * 1_024];
    for index in 0..=(MAX_NATIVE_SKILL_DISCOVERY_BYTES / text.len()) {
        fixture.write(&format!("skills/item-{index:04}/SKILL.md"), &text);
    }
    let snapshot = fixture.snapshot();
    assert!(!snapshot.complete());
    assert_eq!(
        snapshot.entries().len(),
        MAX_NATIVE_SKILL_DISCOVERY_BYTES / text.len()
    );
    assert!(
        snapshot
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.cause == NativeSkillCatalogError::ResourceLimit)
    );
}

#[test]
fn retained_snapshot_text_has_an_independent_bound() {
    let fixture = Fixture::new();
    let description = "d".repeat(crate::skills_metadata::MAX_NATIVE_SKILL_DESCRIPTION_BYTES);
    for index in 0..600 {
        fixture.write(
            &format!("skills/item-{index:04}/SKILL.md"),
            format!("---\nname: item-{index:04}\ndescription: {description}\n---\n"),
        );
    }
    let snapshot = fixture.snapshot();
    assert!(!snapshot.complete());
    assert!(snapshot.entries().len() < 600);
    let bytes: usize = snapshot
        .entries()
        .iter()
        .map(|entry| {
            entry.selection.retained_bytes()
                + entry.metadata.name.len()
                + entry.metadata.description.len()
        })
        .sum();
    assert!(bytes <= MAX_NATIVE_SKILL_SNAPSHOT_BYTES);
}

#[test]
fn exact_full_file_limit_succeeds_and_fifo_is_rejected_without_waiting() {
    let fixture = Fixture::new();
    fixture.write(
        "skills/full/SKILL.md",
        vec![b'x'; MAX_NATIVE_SKILL_MATERIALIZED_BYTES],
    );
    let catalog = fixture.catalog();
    let selected = catalog
        .discover(&CancellationToken::new())
        .unwrap()
        .resolve("full", None)
        .unwrap();
    assert_eq!(
        catalog
            .materialize(&selected, &CancellationToken::new())
            .unwrap()
            .text
            .len(),
        MAX_NATIVE_SKILL_MATERIALIZED_BYTES
    );
    fs::create_dir_all(fixture.0.join("skills/fifo")).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(fixture.0.join("skills/fifo/SKILL.md"))
            .status()
            .unwrap()
            .success()
    );
    let snapshot = catalog.discover(&CancellationToken::new()).unwrap();
    assert!(!snapshot.complete());
    assert_eq!(snapshot.entries().len(), 1);
}

#[test]
fn hidden_skill_names_are_visible_but_managed_transaction_prefix_is_reserved() {
    let fixture = Fixture::new();
    fixture.write("skills/.hidden/SKILL.md", "hidden skill");
    fixture.write(
        "skills/.machine-god-skill-transaction/SKILL.md",
        "internal metadata",
    );
    let root = fixture.root(
        "skills",
        NativeSkillSource::Managed,
        NativeSkillLinkPolicy::Reject,
    );
    let snapshot = NativeSkillCatalog::new(vec![root])
        .unwrap()
        .discover(&CancellationToken::new())
        .unwrap();
    assert!(snapshot.complete());
    assert_eq!(snapshot.entries().len(), 1);
    assert_eq!(snapshot.entries()[0].metadata.name, ".hidden");
    assert_eq!(fixture.snapshot().entries().len(), 2);
}
