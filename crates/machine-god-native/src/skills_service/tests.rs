use super::*;
use crate::skills_catalog::{NativeSkillLinkPolicy, NativeSkillRoot, NativeSkillSource};
use crate::skills_managed::{
    NativeSkillGitRequest, NativeSkillGitRunner, NativeSkillItemReceipt,
    NativeSkillManagedErrorKind,
};
use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mg-skills-service-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::create_dir(path.join("state")).unwrap();
        Self(path)
    }
    fn write(&self, path: &str, text: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn root(&self, relative: &str, source: NativeSkillSource) -> NativeSkillRoot {
        NativeSkillRoot::from_directory(
            Arc::new(fs::File::open(&self.0).unwrap()),
            relative.into(),
            self.0.clone(),
            self.0.join(relative),
            source,
            NativeSkillLinkPolicy::Reject,
        )
        .unwrap()
    }
    fn service(&self) -> NativeSkillsService {
        let managed = Arc::new(NativeManagedSkills::open(&self.0.join("state"), None).unwrap());
        NativeSkillsService::new(
            Arc::new(
                NativeSkillCatalog::new(vec![
                    self.root("workspace/skills", NativeSkillSource::WorkspaceShared),
                    managed.catalog_root().unwrap(),
                ])
                .unwrap(),
            ),
            Some(managed),
        )
    }
    fn execute(&self, command: &str) -> NativeSkillsServiceResult {
        self.service()
            .execute(command.parse().unwrap(), &self.0, &CancellationToken::new())
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn view(result: NativeSkillsServiceResult) -> NativeSkillsCatalogView {
    match result {
        NativeSkillsServiceResult::Catalog(view) => *view,
        _ => panic!("catalog result expected"),
    }
}

#[test]
fn list_defaults_and_path_do_not_create_managed_storage() {
    let fixture = Fixture::new();
    fixture.write("workspace/skills/review/SKILL.md", "body");
    for command in ["", "list"] {
        let result = view(fixture.execute(command));
        assert_eq!(result.snapshot.entries().len(), 1);
        assert!(result.focus.is_none());
        assert!(result.query.is_empty());
    }
    match fixture.execute("path") {
        NativeSkillsServiceResult::Path(path) => assert_eq!(path, fixture.0.join("state/skills")),
        _ => panic!("path result expected"),
    }
    assert!(!fixture.0.join("state/skills").exists());
}

#[test]
fn show_focuses_one_row_without_materializing_body_and_exposes_duplicates() {
    let fixture = Fixture::new();
    fixture.write(
        "workspace/skills/review/SKILL.md",
        "---\nname: review\n---\n",
    );
    let file = fs::OpenOptions::new()
        .write(true)
        .open(fixture.0.join("workspace/skills/review/SKILL.md"))
        .unwrap();
    file.set_len(2 * 1024 * 1024).unwrap();
    let single = view(fixture.execute("show review"));
    assert_eq!(single.focus.unwrap().name(), "review");
    assert!(single.notice.is_none());
    fixture.write(
        "state/skills/review/SKILL.md",
        "---\nname: review\n---\nmanaged",
    );
    let duplicate = view(fixture.execute("show review"));
    assert_eq!(duplicate.snapshot.entries().len(), 2);
    assert!(duplicate.focus.is_none());
    assert_eq!(duplicate.notice, Some(NativeSkillsNotice::Ambiguous));
    assert_eq!(duplicate.query, "review");
    let missing = fixture.execute("show absent");
    assert!(missing.failed());
    assert_eq!(view(missing).notice, Some(NativeSkillsNotice::NotFound));
    fixture.write(
        "state/skills/unicode/SKILL.md",
        "---\nname: name\u{a0}\n---\nbody",
    );
    assert_eq!(
        view(fixture.execute("show name\u{a0}"))
            .focus
            .unwrap()
            .name(),
        "name\u{a0}"
    );
}

#[test]
fn create_replacement_and_install_aliases_preserve_typed_receipts() {
    let fixture = Fixture::new();
    assert!(!fixture.execute("create review").failed());
    fixture.write("state/skills/review/assets/example", "keep");
    let error = fixture
        .service()
        .execute(
            "create review".parse().unwrap(),
            &fixture.0,
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        NativeSkillsServiceError::Managed(NativeSkillManagedError {
            kind: NativeSkillManagedErrorKind::ReplacementConsentRequired,
            ..
        })
    ));
    assert!(!fixture.execute("create review --replace").failed());
    assert_eq!(
        fs::read_to_string(fixture.0.join("state/skills/review/assets/example")).unwrap(),
        "keep"
    );
    fixture.write("incoming/first/SKILL.md", "---\nname: First\n---\nfirst");
    fixture.write("incoming/second/SKILL.md", "second");
    assert!(!fixture.execute("add ./incoming --skill First").failed());
    assert!(
        !fixture
            .execute("install ./incoming --skill=second")
            .failed()
    );
    assert!(fixture.0.join("state/skills/first/SKILL.md").exists());
    assert!(fixture.0.join("state/skills/second/SKILL.md").exists());
}

#[test]
fn remove_prefers_managed_after_workspace_duplicate_and_supports_basename() {
    let fixture = Fixture::new();
    fixture.write(
        "workspace/skills/review/SKILL.md",
        "---\nname: review\n---\nworkspace",
    );
    fixture.write(
        "state/skills/destination/SKILL.md",
        "---\nname: review\n---\nmanaged",
    );
    assert!(!fixture.execute("remove review").failed());
    assert!(fixture.0.join("workspace/skills/review/SKILL.md").exists());
    assert!(!fixture.0.join("state/skills/destination").exists());
    fixture.write(
        "state/skills/destination/SKILL.md",
        "---\nname: advertised\n---\nmanaged",
    );
    assert!(!fixture.execute("remove destination").failed());
}

#[test]
fn ambiguous_managed_names_require_exact_location_and_foreign_roots_are_read_only() {
    let fixture = Fixture::new();
    for basename in ["first", "second"] {
        fixture.write(
            &format!("state/skills/{basename}/SKILL.md"),
            "---\nname: duplicate\n---\nbody",
        );
    }
    let service = fixture.service();
    assert_eq!(
        service
            .execute(
                "remove duplicate".parse().unwrap(),
                &fixture.0,
                &CancellationToken::new()
            )
            .unwrap_err(),
        NativeSkillsServiceError::Catalog(NativeSkillCatalogError::AmbiguousName)
    );
    assert!(
        !service
            .execute(
                NativeSkillsCommand::Remove {
                    selector: fixture
                        .0
                        .join("state/skills/first")
                        .to_str()
                        .unwrap()
                        .into()
                },
                &fixture.0,
                &CancellationToken::new()
            )
            .unwrap()
            .failed()
    );
    assert!(fixture.0.join("state/skills/second").exists());
    fixture.write("foreign/skills/other/SKILL.md", "body");
    let catalog = NativeSkillCatalog::new(vec![
        fixture.root("foreign/skills", NativeSkillSource::Managed),
    ])
    .unwrap();
    let service = NativeSkillsService::new(Arc::new(catalog), fixture.service().managed);
    assert_eq!(
        service
            .execute(
                "remove other".parse().unwrap(),
                &fixture.0,
                &CancellationToken::new()
            )
            .unwrap_err(),
        NativeSkillsServiceError::Catalog(NativeSkillCatalogError::NotFound)
    );
    assert!(fixture.0.join("foreign/skills/other/SKILL.md").exists());
}

#[test]
fn incomplete_discovery_does_not_claim_name_uniqueness_but_exact_managed_location_works() {
    let fixture = Fixture::new();
    fixture.write("workspace/skills/broken/SKILL.md", "---\nname: broken");
    fixture.write("state/skills/valid/SKILL.md", "body");
    let shown = view(fixture.execute("show valid"));
    assert!(!shown.snapshot.complete());
    assert!(shown.focus.is_none());
    assert_eq!(shown.notice, Some(NativeSkillsNotice::IncompleteDiscovery));
    let service = fixture.service();
    assert_eq!(
        service
            .execute(
                "remove valid".parse().unwrap(),
                &fixture.0,
                &CancellationToken::new()
            )
            .unwrap_err(),
        NativeSkillsServiceError::Catalog(NativeSkillCatalogError::IncompleteDiscovery)
    );
    let selector = fixture
        .0
        .join("state/skills/valid")
        .to_str()
        .unwrap()
        .to_owned();
    let shown = view(
        service
            .execute(
                NativeSkillsCommand::Show {
                    selector: selector.clone(),
                },
                &fixture.0,
                &CancellationToken::new(),
            )
            .unwrap(),
    );
    assert!(shown.focus.is_some());
    assert!(!shown.snapshot.complete());
    assert!(
        !service
            .execute(
                NativeSkillsCommand::Remove { selector },
                &fixture.0,
                &CancellationToken::new()
            )
            .unwrap()
            .failed()
    );
}

#[test]
fn direct_enum_validation_and_missing_authority_precede_effects() {
    let fixture = Fixture::new();
    let service =
        NativeSkillsService::new(Arc::new(NativeSkillCatalog::new(Vec::new()).unwrap()), None);
    for command in ["path", "create name", "add ./source", "remove name"] {
        assert_eq!(
            service
                .execute(
                    command.parse().unwrap(),
                    &fixture.0,
                    &CancellationToken::new()
                )
                .unwrap_err(),
            NativeSkillsServiceError::MissingManagedAuthority
        );
    }
    for command in [
        NativeSkillsCommand::Show {
            selector: "../name".into(),
        },
        NativeSkillsCommand::Remove {
            selector: "name\n".into(),
        },
        NativeSkillsCommand::Create {
            arguments: "name\n".into(),
        },
        NativeSkillsCommand::Install {
            arguments: "x".repeat(MAX_NATIVE_SKILLS_COMMAND_BYTES + 1),
        },
    ] {
        assert_eq!(
            service
                .execute(command, &fixture.0, &CancellationToken::new())
                .unwrap_err(),
            NativeSkillsServiceError::InvalidCommand
        );
    }
    assert_eq!(
        service
            .execute(
                "add ./source".parse().unwrap(),
                Path::new("relative"),
                &CancellationToken::new()
            )
            .unwrap_err(),
        NativeSkillsServiceError::InvalidCommand
    );
    assert!(!fixture.0.join("state/skills").exists());
}

#[test]
fn cancelled_commands_do_not_discover_create_or_remove() {
    let fixture = Fixture::new();
    let token = CancellationToken::new();
    token.cancel();
    for command in [
        "list",
        "show name",
        "path",
        "create name",
        "add ./source",
        "remove name",
    ] {
        assert_eq!(
            fixture
                .service()
                .execute(command.parse().unwrap(), &fixture.0, &token)
                .unwrap_err(),
            NativeSkillsServiceError::Cancelled
        );
    }
    assert!(!fixture.0.join("state/skills").exists());
}

#[test]
fn replacement_before_or_after_removal_preparation_cannot_redirect_selection() {
    for before in [true, false] {
        let fixture = Fixture::new();
        fixture.write("state/skills/review/SKILL.md", "original");
        let destination = fixture.0.join("state/skills/review/SKILL.md");
        let hook = move || {
            fs::write(destination, "---\nname: unrelated\n---\nnew").unwrap();
        };
        if before {
            selection::set_prepare_hook(hook);
        } else {
            selection::set_remove_hook(hook);
        }
        assert_eq!(
            fixture
                .service()
                .execute(
                    "remove review".parse().unwrap(),
                    &fixture.0,
                    &CancellationToken::new()
                )
                .unwrap_err(),
            NativeSkillsServiceError::Catalog(NativeSkillCatalogError::StaleSelection)
        );
        assert!(
            fs::read_to_string(fixture.0.join("state/skills/review/SKILL.md"))
                .unwrap()
                .contains("unrelated")
        );
    }
}

#[test]
fn mutation_result_never_misclassifies_partial_receipts_as_success() {
    for outcome in [
        NativeSkillItemOutcome::Failed,
        NativeSkillItemOutcome::RolledBack,
        NativeSkillItemOutcome::Indeterminate,
        NativeSkillItemOutcome::NotAttempted,
    ] {
        let result = NativeSkillsServiceResult::Managed(NativeSkillBatchReceipt {
            items: vec![
                NativeSkillItemReceipt {
                    destination: "first".into(),
                    outcome: NativeSkillItemOutcome::Installed,
                    error: None,
                    recovery_id: None,
                },
                NativeSkillItemReceipt {
                    destination: "second".into(),
                    outcome,
                    error: None,
                    recovery_id: None,
                },
            ],
        });
        assert!(result.failed());
    }
}

#[derive(Debug)]
struct FakeGit;
impl NativeSkillGitRunner for FakeGit {
    fn clone_repository(
        &self,
        request: NativeSkillGitRequest,
        _: &CancellationToken,
    ) -> Result<(), NativeSkillManagedError> {
        assert_eq!(request.url, "https://github.com/owner/repository.git");
        fs::write(request.directory_path.join("SKILL.md"), "remote").unwrap();
        Ok(())
    }
}
#[test]
fn fake_git_install_handles_pasted_syntax_without_package_manager_or_implicit_consent() {
    let fixture = Fixture::new();
    let service = NativeSkillsService::new(
        Arc::new(NativeSkillCatalog::new(Vec::new()).unwrap()),
        Some(Arc::new(
            NativeManagedSkills::open(&fixture.0.join("state"), Some(Arc::new(FakeGit))).unwrap(),
        )),
    );
    let command = "add npx -y skills add owner/repository --yes";
    assert!(
        !service
            .execute(
                command.parse().unwrap(),
                &fixture.0,
                &CancellationToken::new()
            )
            .unwrap()
            .failed()
    );
    let error = service
        .execute(
            command.parse().unwrap(),
            &fixture.0,
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        NativeSkillsServiceError::Managed(NativeSkillManagedError {
            kind: NativeSkillManagedErrorKind::ReplacementConsentRequired,
            ..
        })
    ));
}

#[test]
fn mutation_receipt_survives_catalog_failure_and_errors_remain_redacted() {
    let fixture = Fixture::new();
    fixture.write("workspace/skills/bad/SKILL.md", "---\nname: invalid");
    let result = fixture.execute("create fine");
    assert!(!result.failed());
    let error = NativeSkillsServiceError::Managed(NativeSkillManagedError::with_recovery(
        NativeSkillManagedErrorKind::Indeterminate,
        "secret-recovery".into(),
    ));
    assert!(!format!("{error:?} {error}").contains("secret-recovery"));
    assert!(
        !format!(
            "{:?}",
            NativeSkillsServiceResult::Path("/secret-path".into())
        )
        .contains("secret-path")
    );
}

#[test]
fn identical_display_paths_do_not_rebind_a_retained_managed_capability() {
    let fixture = Fixture::new();
    fixture.write("state/skills/review/SKILL.md", "original");
    let original = NativeManagedSkills::open(&fixture.0.join("state"), None).unwrap();
    let catalog =
        Arc::new(NativeSkillCatalog::new(vec![original.catalog_root().unwrap()]).unwrap());
    fs::rename(fixture.0.join("state"), fixture.0.join("original-state")).unwrap();
    fixture.write("state/skills/review/SKILL.md", "unrelated replacement");
    let replacement = Arc::new(NativeManagedSkills::open(&fixture.0.join("state"), None).unwrap());
    let service = NativeSkillsService::new(catalog, Some(replacement));
    assert_eq!(
        service
            .execute(
                "remove review".parse().unwrap(),
                &fixture.0,
                &CancellationToken::new()
            )
            .unwrap_err(),
        NativeSkillsServiceError::Catalog(NativeSkillCatalogError::WrongAuthority)
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("state/skills/review/SKILL.md")).unwrap(),
        "unrelated replacement"
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("original-state/skills/review/SKILL.md")).unwrap(),
        "original"
    );
}

#[test]
fn independently_reopened_same_inode_is_not_shared_capability_authority() {
    let fixture = Fixture::new();
    fixture.write("state/skills/review/SKILL.md", "original");
    let first = NativeManagedSkills::open(&fixture.0.join("state"), None).unwrap();
    let second = Arc::new(NativeManagedSkills::open(&fixture.0.join("state"), None).unwrap());
    let service = NativeSkillsService::new(
        Arc::new(NativeSkillCatalog::new(vec![first.catalog_root().unwrap()]).unwrap()),
        Some(second),
    );
    assert_eq!(
        service
            .execute(
                "remove review".parse().unwrap(),
                &fixture.0,
                &CancellationToken::new()
            )
            .unwrap_err(),
        NativeSkillsServiceError::Catalog(NativeSkillCatalogError::WrongAuthority)
    );
    assert!(fixture.0.join("state/skills/review/SKILL.md").exists());
}
