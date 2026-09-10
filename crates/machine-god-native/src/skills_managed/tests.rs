use super::*;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "machine-god-skills-managed-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::create_dir(path.join("state")).unwrap();
        fs::create_dir(path.join("source")).unwrap();
        Self { path }
    }
    fn manager(&self) -> NativeManagedSkills {
        NativeManagedSkills::open(&self.path.join("state"), None).unwrap()
    }
    fn source(&self) -> NativeSkillInstallSource {
        NativeSkillInstallSource::parse(self.path.join("source").to_str().unwrap(), None).unwrap()
    }
    fn write(&self, path: &str, content: &str) {
        let destination = self.path.join(path);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, content).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap();
    }
}

#[test]
fn source_normalization_is_effect_free_and_never_promotes_yes_to_replacement() {
    for input in [
        "owner/repo@review",
        "https://skills.sh/owner/repo/review",
        "npx skills add owner/repo --skill review -g -y",
        "npx -y skills add owner/repo --skill review",
        "bunx skills add owner/repo --skill=review --yes",
    ] {
        let source = NativeSkillInstallSource::parse(input, None).unwrap();
        assert_eq!(source.kind(), NativeSkillSourceKind::Git);
        assert_eq!(source.source(), "https://github.com/owner/repo.git");
        assert_eq!(source.filter(), Some("review"));
        let (_, replace) = parse_skill_install_command(input).unwrap();
        assert!(!replace);
    }
    assert!(
        parse_skill_install_command("owner/repo --replace")
            .unwrap()
            .1
    );
    assert_eq!(
        NativeSkillInstallSource::parse("./owner/repo", None)
            .unwrap()
            .kind(),
        NativeSkillSourceKind::Local
    );
    assert_eq!(
        NativeSkillInstallSource::parse("owner/repo@one", Some("two")),
        Err(NativeSkillManagedErrorKind::ConflictingFilter)
    );
}

#[test]
fn malicious_and_ambiguous_source_syntax_is_rejected_before_effects() {
    for source in [
        "",
        "-bad",
        "ext::sh -c command",
        "file:///tmp/repo",
        "https://x.example/repo#fragment",
        "https://user:secret@x.example/repo",
        "npx skills add owner/repo ; touch /tmp/no",
        "npx skills add owner/repo --unknown",
        "https://skills.sh/a/b/c/d",
    ] {
        assert!(
            NativeSkillInstallSource::parse(source, None).is_err(),
            "{source}"
        );
    }
    assert!(parse_skill_install_command("owner/repo other/repo").is_err());
}

#[test]
fn all_selected_skills_are_installed_deterministically_with_resources() {
    let fixture = Fixture::new();
    fixture.write("source/zeta/SKILL.md", "---\nname: Zeta\n---\nbody");
    fixture.write("source/alpha/SKILL.md", "alpha body");
    fixture.write("source/alpha/assets/item.txt", "resource");
    let owner = fixture.manager();
    let cancellation = CancellationToken::new();
    let plan = owner
        .prepare_install(&fixture.source(), &fixture.path, &cancellation)
        .unwrap();
    assert_eq!(
        plan.items()
            .iter()
            .map(|item| item.destination.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "zeta"]
    );
    assert!(!fixture.path.join("state/skills").exists());
    let receipt = owner
        .commit(
            plan,
            &NativeSkillReplacementConsent::NoReplace,
            &cancellation,
        )
        .unwrap();
    assert!(
        receipt
            .items
            .iter()
            .all(|item| item.outcome == NativeSkillItemOutcome::Installed)
    );
    assert_eq!(
        fs::read_to_string(fixture.path.join("state/skills/alpha/assets/item.txt")).unwrap(),
        "resource"
    );
}

#[test]
fn replacing_requires_exact_consent_and_rejects_changed_destination() {
    let fixture = Fixture::new();
    fixture.write("source/review/SKILL.md", "new");
    fixture.write("state/skills/review/SKILL.md", "old");
    let owner = fixture.manager();
    let cancellation = CancellationToken::new();
    let plan = owner
        .prepare_install(&fixture.source(), &fixture.path, &cancellation)
        .unwrap();
    assert_eq!(
        owner
            .commit(
                plan,
                &NativeSkillReplacementConsent::NoReplace,
                &cancellation
            )
            .unwrap_err()
            .kind,
        NativeSkillManagedErrorKind::ReplacementConsentRequired
    );
    let plan = owner
        .prepare_install(&fixture.source(), &fixture.path, &cancellation)
        .unwrap();
    let consent = NativeSkillReplacementConsent::ExactDestinations(plan.replacements());
    fixture.write("state/skills/review/SKILL.md", "raced");
    assert_eq!(
        owner
            .commit(plan, &consent, &cancellation)
            .unwrap_err()
            .kind,
        NativeSkillManagedErrorKind::Changed
    );
    assert_eq!(
        fs::read_to_string(fixture.path.join("state/skills/review/SKILL.md")).unwrap(),
        "raced"
    );
    let plan = owner
        .prepare_install(&fixture.source(), &fixture.path, &cancellation)
        .unwrap();
    let consent = NativeSkillReplacementConsent::ExactDestinations(plan.replacements());
    assert_eq!(
        owner.commit(plan, &consent, &cancellation).unwrap().items[0].outcome,
        NativeSkillItemOutcome::Replaced
    );
}

#[test]
fn source_change_is_rejected_before_managed_publication() {
    let fixture = Fixture::new();
    fixture.write("source/review/SKILL.md", "before");
    let owner = fixture.manager();
    let cancellation = CancellationToken::new();
    let plan = owner
        .prepare_install(&fixture.source(), &fixture.path, &cancellation)
        .unwrap();
    fixture.write("source/review/SKILL.md", "after");
    assert_eq!(
        owner
            .commit(
                plan,
                &NativeSkillReplacementConsent::NoReplace,
                &cancellation
            )
            .unwrap_err()
            .kind,
        NativeSkillManagedErrorKind::Changed
    );
    assert!(!fixture.path.join("state/skills").exists());
}

#[test]
fn create_preserves_siblings_and_remove_uses_exact_managed_revision() {
    let fixture = Fixture::new();
    fixture.write("state/skills/review/SKILL.md", "old");
    fixture.write("state/skills/review/assets/file", "keep");
    let owner = fixture.manager();
    let cancellation = CancellationToken::new();
    let plan = owner.prepare_create("review", &cancellation).unwrap();
    let consent = NativeSkillReplacementConsent::ExactDestinations(plan.replacements());
    let receipt = owner.commit(plan, &consent, &cancellation).unwrap();
    assert_eq!(receipt.items[0].outcome, NativeSkillItemOutcome::Replaced);
    assert_eq!(
        fs::read_to_string(fixture.path.join("state/skills/review/assets/file")).unwrap(),
        "keep"
    );
    let plan = owner.prepare_remove("review", &cancellation).unwrap();
    assert_eq!(
        owner
            .commit(
                plan,
                &NativeSkillReplacementConsent::NoReplace,
                &cancellation
            )
            .unwrap()
            .items[0]
            .outcome,
        NativeSkillItemOutcome::Removed
    );
    assert!(!fixture.path.join("state/skills/review").exists());
}

#[test]
fn collisions_are_preflighted_before_any_destination_effect() {
    let fixture = Fixture::new();
    fixture.write("source/a/review/SKILL.md", "a");
    fixture.write("source/b/Review/SKILL.md", "b");
    assert_eq!(
        fixture
            .manager()
            .prepare_install(&fixture.source(), &fixture.path, &CancellationToken::new())
            .unwrap_err()
            .kind,
        NativeSkillManagedErrorKind::Collision
    );
    assert!(!fixture.path.join("state/skills").exists());
}

#[test]
fn filter_matches_metadata_or_basename_and_rejects_incomplete_metadata() {
    let fixture = Fixture::new();
    fixture.write(
        "source/location/SKILL.md",
        "---\nname: Advertised\n---\nbody",
    );
    for filter in ["location", "Advertised"] {
        let source = NativeSkillInstallSource::parse(
            fixture.path.join("source").to_str().unwrap(),
            Some(filter),
        )
        .unwrap();
        assert_eq!(
            fixture
                .manager()
                .prepare_install(&source, &fixture.path, &CancellationToken::new())
                .unwrap()
                .items()
                .len(),
            1
        );
    }
    fixture.write("source/location/SKILL.md", "---\nname: broken\n");
    assert_eq!(
        fixture
            .manager()
            .prepare_install(&fixture.source(), &fixture.path, &CancellationToken::new())
            .unwrap_err()
            .kind,
        NativeSkillManagedErrorKind::InvalidMetadata
    );
}

#[test]
fn cancellation_and_foreign_plans_never_publish() {
    let fixture = Fixture::new();
    let owner = fixture.manager();
    let cancellation = CancellationToken::new();
    let plan = owner.prepare_create("review", &cancellation).unwrap();
    cancellation.cancel();
    assert_eq!(
        owner
            .commit(
                plan,
                &NativeSkillReplacementConsent::NoReplace,
                &cancellation
            )
            .unwrap_err()
            .kind,
        NativeSkillManagedErrorKind::Cancelled
    );
    assert!(!fixture.path.join("state/skills").exists());
    let token = CancellationToken::new();
    let plan = owner.prepare_create("review", &token).unwrap();
    assert_eq!(
        fixture
            .manager()
            .commit(plan, &NativeSkillReplacementConsent::NoReplace, &token)
            .unwrap_err()
            .kind,
        NativeSkillManagedErrorKind::Changed
    );
}

#[test]
fn symlinks_and_oversized_files_fail_closed() {
    let fixture = Fixture::new();
    fixture.write("source/review/SKILL.md", "body");
    std::os::unix::fs::symlink("/etc/passwd", fixture.path.join("source/review/linked")).unwrap();
    assert_eq!(
        fixture
            .manager()
            .prepare_install(&fixture.source(), &fixture.path, &CancellationToken::new())
            .unwrap_err()
            .kind,
        NativeSkillManagedErrorKind::InvalidEntry
    );
    fs::remove_file(fixture.path.join("source/review/linked")).unwrap();
    let file = File::create(fixture.path.join("source/review/large")).unwrap();
    file.set_len((MAX_MANAGED_SKILL_FILE_BYTES + 1) as u64)
        .unwrap();
    assert_eq!(
        fixture
            .manager()
            .prepare_install(&fixture.source(), &fixture.path, &CancellationToken::new())
            .unwrap_err()
            .kind,
        NativeSkillManagedErrorKind::ResourceLimit
    );
}

#[derive(Debug)]
struct FakeGit {
    calls: Arc<AtomicUsize>,
}
impl NativeSkillGitRunner for FakeGit {
    fn clone_repository(
        &self,
        request: NativeSkillGitRequest,
        _: &CancellationToken,
    ) -> Result<(), NativeSkillManagedError> {
        assert_eq!(request.url, "https://github.com/owner/repo.git");
        assert_eq!(request.max_output_bytes, 64 * 1024);
        self.calls.fetch_add(1, Ordering::Relaxed);
        fs::write(request.directory_path.join("SKILL.md"), "from Git").unwrap();
        Ok(())
    }
}
#[test]
fn explicit_runner_receives_normalized_git_and_clone_is_cleaned_before_consent() {
    let fixture = Fixture::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let owner = NativeManagedSkills::open(
        &fixture.path.join("state"),
        Some(Arc::new(FakeGit {
            calls: Arc::clone(&calls),
        })),
    )
    .unwrap();
    let source = NativeSkillInstallSource::parse("owner/repo", None).unwrap();
    let token = CancellationToken::new();
    let plan = owner
        .prepare_install(&source, &fixture.path, &token)
        .unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(fs::read_dir(fixture.path.join("state")).unwrap().count(), 0);
    let receipt = owner
        .commit(plan, &NativeSkillReplacementConsent::NoReplace, &token)
        .unwrap();
    assert_eq!(receipt.items[0].outcome, NativeSkillItemOutcome::Installed);
}

#[test]
fn errors_and_requests_do_not_debug_print_recovery_paths_or_secrets() {
    let error = NativeSkillManagedError::with_recovery(
        NativeSkillManagedErrorKind::Indeterminate,
        "secret-path".into(),
    );
    assert!(!format!("{error:?} {error}").contains("secret-path"));
    assert!(
        !format!(
            "{:?}",
            NativeSkillInstallSource::parse("https://example.test/private", None).unwrap()
        )
        .contains("private")
    );
}

#[test]
fn failed_replacement_rolls_back_exact_original_and_cleans_transaction() {
    let fixture = Fixture::new();
    fixture.write("state/skills/review/SKILL.md", "old");
    let owner = fixture.manager();
    let token = CancellationToken::new();
    let plan = owner.prepare_create("review", &token).unwrap();
    let consent = NativeSkillReplacementConsent::ExactDestinations(plan.replacements());
    publication::inject_fault(publication::InjectedFault::Publish);
    let receipt = owner.commit(plan, &consent, &token).unwrap();
    assert_eq!(receipt.items[0].outcome, NativeSkillItemOutcome::RolledBack);
    assert!(receipt.items[0].recovery_id.is_none());
    assert_eq!(
        fs::read_to_string(fixture.path.join("state/skills/review/SKILL.md")).unwrap(),
        "old"
    );
    assert_eq!(
        fs::read_dir(fixture.path.join("state/skills"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn uncertain_rollback_preserves_backup_staging_and_exact_recovery_identifier() {
    let fixture = Fixture::new();
    fixture.write("state/skills/review/SKILL.md", "old");
    let owner = fixture.manager();
    let token = CancellationToken::new();
    let plan = owner.prepare_create("review", &token).unwrap();
    let consent = NativeSkillReplacementConsent::ExactDestinations(plan.replacements());
    publication::inject_fault(publication::InjectedFault::PublishAndRollback);
    let receipt = owner.commit(plan, &consent, &token).unwrap();
    assert_eq!(
        receipt.items[0].outcome,
        NativeSkillItemOutcome::Indeterminate
    );
    let transaction = fixture
        .path
        .join("state/skills")
        .join(receipt.items[0].recovery_id.as_ref().unwrap());
    assert_eq!(
        fs::read_to_string(transaction.join("backup/SKILL.md")).unwrap(),
        "old"
    );
    assert!(transaction.join("staged/SKILL.md").exists());
    assert!(!fixture.path.join("state/skills/review").exists());
}

#[test]
fn uncertain_postcommit_validation_sync_and_cleanup_never_claim_success() {
    for fault in [
        publication::InjectedFault::PostPublication,
        publication::InjectedFault::Sync,
        publication::InjectedFault::Cleanup,
    ] {
        let fixture = Fixture::new();
        fixture.write("state/skills/review/SKILL.md", "old");
        let owner = fixture.manager();
        let token = CancellationToken::new();
        let plan = owner.prepare_create("review", &token).unwrap();
        let consent = NativeSkillReplacementConsent::ExactDestinations(plan.replacements());
        publication::inject_fault(fault);
        let receipt = owner.commit(plan, &consent, &token).unwrap();
        assert_eq!(
            receipt.items[0].outcome,
            NativeSkillItemOutcome::Indeterminate
        );
        assert!(fixture.path.join("state/skills/review/SKILL.md").exists());
        let transaction = fixture
            .path
            .join("state/skills")
            .join(receipt.items[0].recovery_id.as_ref().unwrap());
        assert_eq!(
            fs::read_to_string(transaction.join("backup/SKILL.md")).unwrap(),
            "old"
        );
    }
}

#[test]
fn uncertain_first_item_stops_batch_with_not_attempted_receipt() {
    let fixture = Fixture::new();
    fixture.write("source/alpha/SKILL.md", "a");
    fixture.write("source/zeta/SKILL.md", "z");
    let owner = fixture.manager();
    let token = CancellationToken::new();
    let plan = owner
        .prepare_install(&fixture.source(), &fixture.path, &token)
        .unwrap();
    publication::inject_fault(publication::InjectedFault::PostPublication);
    let receipt = owner
        .commit(plan, &NativeSkillReplacementConsent::NoReplace, &token)
        .unwrap();
    assert_eq!(
        receipt.items[0].outcome,
        NativeSkillItemOutcome::Indeterminate
    );
    assert_eq!(
        receipt.items[1].outcome,
        NativeSkillItemOutcome::NotAttempted
    );
    assert!(!fixture.path.join("state/skills/zeta").exists());
}

#[test]
fn cancellation_after_backup_restores_existing_destination() {
    let fixture = Fixture::new();
    fixture.write("state/skills/review/SKILL.md", "old");
    let owner = fixture.manager();
    let token = CancellationToken::new();
    let plan = owner.prepare_create("review", &token).unwrap();
    let consent = NativeSkillReplacementConsent::ExactDestinations(plan.replacements());
    publication::inject_fault(publication::InjectedFault::CancelAfterBackup);
    let receipt = owner.commit(plan, &consent, &token).unwrap();
    assert_eq!(receipt.items[0].outcome, NativeSkillItemOutcome::RolledBack);
    assert_eq!(
        receipt.items[0].error,
        Some(NativeSkillManagedErrorKind::Cancelled)
    );
    assert_eq!(
        fs::read_to_string(fixture.path.join("state/skills/review/SKILL.md")).unwrap(),
        "old"
    );
}

#[derive(Debug)]
struct DeferredGit {
    retained: Arc<std::sync::Mutex<Option<NativeSkillGitRequest>>>,
}
impl NativeSkillGitRunner for DeferredGit {
    fn clone_repository(
        &self,
        request: NativeSkillGitRequest,
        _: &CancellationToken,
    ) -> Result<(), NativeSkillManagedError> {
        *self.retained.lock().unwrap() = Some(request);
        Err(NativeSkillManagedErrorKind::Cancelled.into())
    }
}

#[test]
fn failed_git_retains_clone_until_process_lease_is_released() {
    let fixture = Fixture::new();
    let retained = Arc::new(std::sync::Mutex::new(None));
    let owner = NativeManagedSkills::open(
        &fixture.path.join("state"),
        Some(Arc::new(DeferredGit {
            retained: Arc::clone(&retained),
        })),
    )
    .unwrap();
    let error = owner
        .prepare_install(
            &NativeSkillInstallSource::parse("owner/repo", None).unwrap(),
            &fixture.path,
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert_eq!(error.kind, NativeSkillManagedErrorKind::Cancelled);
    let transaction = fixture.path.join("state").join(error.recovery_id.unwrap());
    assert!(transaction.exists());
    drop(retained.lock().unwrap().take());
    assert!(!transaction.exists());
}

#[test]
fn managed_namespace_replacement_invalidates_plan() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.path.join("state/skills")).unwrap();
    let owner = fixture.manager();
    let token = CancellationToken::new();
    let plan = owner.prepare_create("new", &token).unwrap();
    fs::rename(
        fixture.path.join("state/skills"),
        fixture.path.join("old-skills"),
    )
    .unwrap();
    fs::create_dir(fixture.path.join("state/skills")).unwrap();
    assert_eq!(
        owner
            .commit(plan, &NativeSkillReplacementConsent::NoReplace, &token)
            .unwrap_err()
            .kind,
        NativeSkillManagedErrorKind::Changed
    );
    assert_eq!(
        fs::read_dir(fixture.path.join("state/skills"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn hidden_and_quoted_names_preserve_pinned_metadata_but_internal_namespace_is_reserved() {
    let fixture = Fixture::new();
    let owner = fixture.manager();
    let token = CancellationToken::new();
    for name in [".hidden", "both'quotes\"", "Space Name", "résumé"] {
        let plan = owner.prepare_create(name, &token).unwrap();
        assert_eq!(
            owner
                .commit(plan, &NativeSkillReplacementConsent::NoReplace, &token)
                .unwrap()
                .items[0]
                .outcome,
            NativeSkillItemOutcome::Installed
        );
        let bytes = fs::read(
            fixture
                .path
                .join("state/skills")
                .join(name)
                .join("SKILL.md"),
        )
        .unwrap();
        assert_eq!(
            crate::skills_metadata::parse_skill_metadata(&bytes, name)
                .unwrap()
                .name,
            name
        );
    }
    assert!(
        owner
            .prepare_create(".machine-god-skill-user", &token)
            .is_err()
    );
    assert!(
        owner
            .prepare_create(".Machine-God-Skill-user", &token)
            .is_err()
    );
}

#[test]
fn local_source_failure_never_falls_back_to_git() {
    let fixture = Fixture::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let owner = NativeManagedSkills::open(
        &fixture.path.join("state"),
        Some(Arc::new(FakeGit {
            calls: Arc::clone(&calls),
        })),
    )
    .unwrap();
    assert!(
        owner
            .prepare_install(
                &NativeSkillInstallSource::parse("./missing/repo", None).unwrap(),
                &fixture.path,
                &CancellationToken::new()
            )
            .is_err()
    );
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn cancelled_clone_cleanup_retains_original_worker_completion_attribution() {
    let fixture = Fixture::new();
    let retained = Arc::new(std::sync::Mutex::new(None));
    let owner = NativeManagedSkills::open(
        &fixture.path.join("state"),
        Some(Arc::new(DeferredGit {
            retained: Arc::clone(&retained),
        })),
    )
    .unwrap();
    let scope = crate::NativeOwnedWorkerScope::new();
    let completion = scope.completion();
    let cwd = fixture.path.clone();
    let error = futures_executor::block_on(scope.run(move || {
        owner.prepare_install(
            &NativeSkillInstallSource::parse("owner/repo", None).unwrap(),
            &cwd,
            &CancellationToken::new(),
        )
    }))
    .unwrap()
    .unwrap_err();
    scope.close();
    assert!(!completion.is_complete());
    assert!(
        fixture
            .path
            .join("state")
            .join(error.recovery_id.unwrap())
            .exists()
    );
    drop(retained.lock().unwrap().take());
    completion.wait_on_worker().unwrap();
    assert!(completion.is_complete());
}

#[test]
fn managed_file_entry_and_selection_limits_are_finite() {
    let fixture = Fixture::new();
    for index in 0..=MAX_MANAGED_SKILL_ITEMS {
        fixture.write(&format!("source/skill-{index:03}/SKILL.md"), "body");
    }
    assert_eq!(
        fixture
            .manager()
            .prepare_install(&fixture.source(), &fixture.path, &CancellationToken::new())
            .unwrap_err()
            .kind,
        NativeSkillManagedErrorKind::ResourceLimit
    );
    assert!(!fixture.path.join("state/skills").exists());
}

#[test]
fn url_query_credentials_never_become_destination_names() {
    let source =
        NativeSkillInstallSource::parse("https://example.test/owner/repo.git?token=secret", None)
            .unwrap();
    assert_eq!(
        planning::source_root_name(&source, Path::new("/workspace")).unwrap(),
        "repo"
    );
    assert!(NativeSkillInstallSource::parse("ssh://git@[::1]/owner/repo.git", None).is_ok());
    let source =
        NativeSkillInstallSource::parse("https://example.test/repo.git.git", None).unwrap();
    assert_eq!(
        planning::source_root_name(&source, Path::new("/workspace")).unwrap(),
        "repo.git"
    );
}

#[test]
fn installing_dot_uses_explicit_cwd_basename() {
    let fixture = Fixture::new();
    fixture.write("source/SKILL.md", "body");
    let owner = fixture.manager();
    let token = CancellationToken::new();
    let plan = owner
        .prepare_install(
            &NativeSkillInstallSource::parse(".", None).unwrap(),
            &fixture.path.join("source"),
            &token,
        )
        .unwrap();
    assert_eq!(plan.items()[0].destination, "source");
    assert_eq!(
        owner
            .commit(plan, &NativeSkillReplacementConsent::NoReplace, &token)
            .unwrap()
            .items[0]
            .outcome,
        NativeSkillItemOutcome::Installed
    );
}
