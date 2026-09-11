use super::Fixture;
use crate::skills_managed::{
    MAX_MANAGED_SKILL_FILE_BYTES, MAX_MANAGED_SKILL_TOTAL_BYTES, NativeManagedSkills,
    NativeSkillGitRequest, NativeSkillGitRunner, NativeSkillInstallSource, NativeSkillItemOutcome,
    NativeSkillManagedError, NativeSkillManagedErrorKind as Error, NativeSkillReplacementConsent,
};
use machine_god_core::CancellationToken;
use std::{fs, os::unix::fs::PermissionsExt, path::Path, sync::Arc};

fn hidden_source(root: &Path) {
    let skill = root.join(".github/skills/review");
    fs::create_dir_all(skill.join("scripts")).unwrap();
    fs::write(skill.join("SKILL.md"), "---\nname: Review code\n---\nbody").unwrap();
    fs::write(skill.join("scripts/run.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(
        skill.join("scripts/run.sh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fs::create_dir_all(skill.join(".git/resources")).unwrap();
    fs::write(skill.join(".git/resources/excluded"), "excluded").unwrap();
    for path in [
        root.join(".git/objects/pack/large"),
        skill.join(".gitignore"),
        root.join("unselected/large"),
    ] {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::File::create(&path)
            .unwrap()
            .set_len((MAX_MANAGED_SKILL_TOTAL_BYTES + 1) as u64)
            .unwrap();
        // Inventory must stat this file, not open its unreadable resource body.
        fs::set_permissions(path, fs::Permissions::from_mode(0o0)).unwrap();
    }
    std::os::unix::fs::symlink("missing-external-target", root.join("unselected/link")).unwrap();
}

fn assert_hidden_install(
    fixture: &Fixture,
    owner: &NativeManagedSkills,
    source: &NativeSkillInstallSource,
) {
    let token = CancellationToken::new();
    let plan = owner
        .prepare_install(source, &fixture.path, &token)
        .unwrap();
    assert_eq!(plan.items().len(), 1);
    assert_eq!(plan.items()[0].name, "Review code");
    assert_eq!(plan.items()[0].destination, "review");
    let receipt = owner
        .commit(plan, &NativeSkillReplacementConsent::NoReplace, &token)
        .unwrap();
    assert_eq!(receipt.items[0].outcome, NativeSkillItemOutcome::Installed);
    let installed = fixture.path.join("state/skills/review");
    assert!(installed.join("SKILL.md").is_file());
    assert_eq!(
        fs::read(installed.join("scripts/run.sh")).unwrap(),
        b"#!/bin/sh\nexit 0\n"
    );
    assert_eq!(
        fs::metadata(installed.join("scripts/run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o711
    );
    assert!(!installed.join(".git").exists());
    assert!(!installed.join(".gitignore").exists());
}

#[test]
fn local_hidden_ancestor_discovery_does_not_read_excluded_or_unselected_bodies() {
    let fixture = Fixture::new();
    hidden_source(&fixture.path.join("source"));
    assert_hidden_install(&fixture, &fixture.manager(), &fixture.source());
}

#[derive(Debug)]
struct HiddenGit;
impl NativeSkillGitRunner for HiddenGit {
    fn clone_repository(
        &self,
        request: NativeSkillGitRequest,
        _: &CancellationToken,
    ) -> Result<(), NativeSkillManagedError> {
        hidden_source(&request.directory_path);
        Ok(())
    }
}

#[test]
fn fake_git_hidden_ancestor_survives_clone_cleanup_and_preserves_copy_exclusions() {
    let fixture = Fixture::new();
    let owner =
        NativeManagedSkills::open(&fixture.path.join("state"), Some(Arc::new(HiddenGit))).unwrap();
    assert_hidden_install(
        &fixture,
        &owner,
        &NativeSkillInstallSource::parse("owner/repo", Some("Review code")).unwrap(),
    );
    assert!(
        fs::read_dir(fixture.path.join("state"))
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".machine-god-skill-"))
    );
}

#[test]
fn all_hidden_ancestors_remain_discoverable_while_selected_root_copy_still_excludes_them() {
    let fixture = Fixture::new();
    fixture.write("source/SKILL.md", "root");
    fixture.write("source/.github/skills/review/SKILL.md", "review");
    fixture.write("source/.git/custom/lint/SKILL.md", "lint");
    fixture.write("source/.hidden/check/SKILL.md", "check");
    let owner = fixture.manager();
    let token = CancellationToken::new();
    let plan = owner
        .prepare_install(&fixture.source(), &fixture.path, &token)
        .unwrap();
    assert_eq!(
        plan.items()
            .iter()
            .map(|item| item.destination.as_str())
            .collect::<Vec<_>>(),
        ["check", "lint", "review", "source"]
    );
    let receipt = owner
        .commit(plan, &NativeSkillReplacementConsent::NoReplace, &token)
        .unwrap();
    assert!(
        receipt
            .items
            .iter()
            .all(|item| item.outcome == NativeSkillItemOutcome::Installed)
    );
    assert!(!fixture.path.join("state/skills/source/.github").exists());
    assert!(!fixture.path.join("state/skills/source/.git").exists());
    assert!(
        fixture
            .path
            .join("state/skills/source/.hidden/check/SKILL.md")
            .is_file()
    );
    assert!(fixture.path.join("state/skills/lint/SKILL.md").is_file());
}

#[test]
fn hidden_candidates_participate_in_filter_validation_and_collision_preflight() {
    let fixture = Fixture::new();
    fixture.write("source/ordinary/review/SKILL.md", "ordinary");
    fixture.write("source/.github/skills/review/SKILL.md", "hidden");
    let owner = fixture.manager();
    let token = CancellationToken::new();
    assert_eq!(
        owner
            .prepare_install(&fixture.source(), &fixture.path, &token)
            .unwrap_err()
            .kind,
        Error::Collision
    );
    fixture.write(
        "source/.github/skills/review/SKILL.md",
        "---\nname:\n---\ninvalid",
    );
    let filtered = NativeSkillInstallSource::parse(
        fixture.path.join("source").to_str().unwrap(),
        Some("ordinary"),
    )
    .unwrap();
    assert_eq!(
        owner
            .prepare_install(&filtered, &fixture.path, &token)
            .unwrap_err()
            .kind,
        Error::InvalidMetadata
    );
    assert!(!fixture.path.join("state/skills").exists());
}

#[test]
fn candidate_resource_mode_readiness_and_membership_changes_reject_stale_plans() {
    for change in 0..7 {
        let fixture = Fixture::new();
        fixture.write("source/.github/skills/review/SKILL.md", "body");
        fixture.write("source/.github/skills/review/run.sh", "script");
        let owner = fixture.manager();
        let token = CancellationToken::new();
        let plan = owner
            .prepare_install(&fixture.source(), &fixture.path, &token)
            .unwrap();
        let skill = fixture.path.join("source/.github/skills/review");
        match change {
            0 => fs::write(skill.join("SKILL.md"), "changed").unwrap(),
            1 => fs::write(skill.join("run.sh"), "changed").unwrap(),
            2 => fs::set_permissions(skill.join("run.sh"), fs::Permissions::from_mode(0o755))
                .unwrap(),
            3 => {
                fs::set_permissions(skill.join("run.sh"), fs::Permissions::from_mode(0o0)).unwrap();
            }
            4 => fixture.write("source/.git/new/review/SKILL.md", "new ambiguous candidate"),
            5 => fs::rename(&skill, skill.with_file_name("renamed")).unwrap(),
            6 => fixture.write("source/unselected/new", "new inventory member"),
            _ => unreachable!(),
        }
        assert_eq!(
            owner
                .commit(plan, &NativeSkillReplacementConsent::NoReplace, &token)
                .unwrap_err()
                .kind,
            Error::Changed
        );
        assert!(!fixture.path.join("state/skills").exists());
    }
}

#[test]
fn overlapping_selected_trees_share_bytes_and_keep_the_aggregate_content_limit() {
    let fixture = Fixture::new();
    fixture.write("source/SKILL.md", "root");
    fixture.write("source/review/SKILL.md", "body");
    for index in 0..32 {
        let path = fixture.path.join(format!("source/review/file-{index:02}"));
        fs::File::create(path)
            .unwrap()
            .set_len((MAX_MANAGED_SKILL_FILE_BYTES - usize::from(index == 0) * 16) as u64)
            .unwrap();
    }
    let owner = fixture.manager();
    let token = CancellationToken::new();
    let plan = owner
        .prepare_install(&fixture.source(), &fixture.path, &token)
        .unwrap();
    let parent = plan
        .items
        .iter()
        .find(|item| item.destination == "source")
        .unwrap();
    let child = plan
        .items
        .iter()
        .find(|item| item.destination == "review")
        .unwrap();
    let parent_bytes = parent
        .tree
        .entries
        .iter()
        .find(|entry| entry.path == "review/file-00")
        .unwrap()
        .bytes
        .as_ref()
        .unwrap();
    let child_bytes = child
        .tree
        .entries
        .iter()
        .find(|entry| entry.path == "file-00")
        .unwrap()
        .bytes
        .as_ref()
        .unwrap();
    assert!(Arc::ptr_eq(parent_bytes, child_bytes));
    assert!(
        plan.items().iter().map(|item| item.bytes).sum::<usize>() <= MAX_MANAGED_SKILL_TOTAL_BYTES
    );
    drop(plan);
    fs::OpenOptions::new()
        .write(true)
        .open(fixture.path.join("source/review/file-00"))
        .unwrap()
        .set_len(MAX_MANAGED_SKILL_FILE_BYTES as u64)
        .unwrap();
    assert_eq!(
        owner
            .prepare_install(&fixture.source(), &fixture.path, &token)
            .unwrap_err()
            .kind,
        Error::ResourceLimit
    );
}

#[test]
fn candidate_inventory_does_not_reduce_exact_selected_byte_capacity() {
    let fixture = Fixture::new();
    fixture.write("source/SKILL.md", &"x".repeat(MAX_MANAGED_SKILL_FILE_BYTES));
    for index in 1..MAX_MANAGED_SKILL_TOTAL_BYTES / MAX_MANAGED_SKILL_FILE_BYTES {
        fs::File::create(fixture.path.join(format!("source/resource-{index:02}")))
            .unwrap()
            .set_len(MAX_MANAGED_SKILL_FILE_BYTES as u64)
            .unwrap();
    }
    let plan = fixture
        .manager()
        .prepare_install(&fixture.source(), &fixture.path, &CancellationToken::new())
        .unwrap();
    assert_eq!(plan.items().len(), 1);
    assert_eq!(plan.items()[0].bytes, MAX_MANAGED_SKILL_TOTAL_BYTES);
}

#[test]
fn hidden_traversal_and_candidate_file_types_remain_bounded_and_no_follow() {
    let fixture = Fixture::new();
    fixture.write("source/.github/skills/review/SKILL.md", "body");
    let owner = fixture.manager();
    let token = CancellationToken::new();
    let path = format!("source/.git/{}/file", "deep/".repeat(33));
    fixture.write(&path, "unselected");
    assert_eq!(
        owner
            .prepare_install(&fixture.source(), &fixture.path, &token)
            .unwrap_err()
            .kind,
        Error::ResourceLimit
    );
    let fixture = Fixture::new();
    fixture.write("source/.github/skills/review/resource", "body");
    std::os::unix::fs::symlink(
        "resource",
        fixture.path.join("source/.github/skills/review/SKILL.md"),
    )
    .unwrap();
    assert_eq!(
        fixture
            .manager()
            .prepare_install(&fixture.source(), &fixture.path, &token)
            .unwrap_err()
            .kind,
        Error::InvalidEntry
    );
}
