use super::{Prepared, environment, prepare};
use machine_god_core::CancellationToken;
use machine_god_native::{
    NativeEnvironment, NativeOwnedWorkerScope, NativeReferenceHostTerminalOptions,
    NativeRootSelection, NativeSkillCatalogError, NativeSkillInvocationPlan,
    NativeSkillManagedErrorKind, NativeSkillSource, NativeSkillsCommand, NativeSkillsServiceError,
    NativeSkillsServiceResult, PreparedNativeRoots, TokioWebSearchDeadline, TokioWebSearchRuntime,
};
use std::{
    ffi::OsString,
    fs,
    os::unix::{ffi::OsStringExt, fs::PermissionsExt},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    base: PathBuf,
    runtime: TokioWebSearchRuntime,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "mg-cli-skills-startup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&base).unwrap();
        let base = fs::canonicalize(base).unwrap();
        for relative in [
            "home",
            "home/project",
            "home/project/workspace",
            "state",
            "empty-bin",
        ] {
            let path = base.join(relative);
            fs::create_dir(&path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let (runtime, _) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
        Self { base, runtime }
    }
    fn values(&self) -> Vec<(OsString, OsString)> {
        vec![
            (
                "XDG_CONFIG_HOME".into(),
                self.base.join("config").into_os_string(),
            ),
            (
                "XDG_STATE_HOME".into(),
                self.base.join("state").into_os_string(),
            ),
            ("HOME".into(), self.base.join("home").into_os_string()),
            ("PATH".into(), self.base.join("empty-bin").into_os_string()),
        ]
    }
    fn roots(&self) -> PreparedNativeRoots {
        PreparedNativeRoots::prepare(
            NativeRootSelection::from_environment(
                &environment(&self.values()),
                &self.base.join("home/project/workspace"),
            )
            .unwrap(),
        )
        .unwrap()
    }
    fn terminal(values: Vec<(OsString, OsString)>) -> NativeReferenceHostTerminalOptions {
        NativeReferenceHostTerminalOptions::new("/bin/false".into(), None, values).unwrap()
    }
    fn prepare(&self, discover: bool) -> Prepared {
        prepare(
            &self.runtime,
            self.roots(),
            environment(&self.values()),
            Self::terminal(self.values()),
            discover,
        )
        .unwrap()
    }
    fn skill(&self, relative: &str, name: &str) -> String {
        let text = format!("---\nname: {name}\ndescription: fixture\n---\nexact {name} body\n");
        self.write(&format!("{relative}/SKILL.md"), &text);
        text
    }
    fn write(&self, relative: &str, text: &str) {
        let path = self.base.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn effect<T: Send + 'static>(&self, operation: impl FnOnce() -> T + Send + 'static) -> T {
        let scope = NativeOwnedWorkerScope::new();
        let result = self.runtime.block_on(scope.run(operation));
        scope.close();
        scope.completion().wait_on_worker().unwrap();
        result.unwrap()
    }
    fn execute(
        &self,
        prepared: &Prepared,
        command: &str,
    ) -> Result<NativeSkillsServiceResult, NativeSkillsServiceError> {
        let service = Arc::clone(&prepared.service);
        let cwd = prepared.roots.workspace_root().to_owned();
        let command: NativeSkillsCommand = command.parse().unwrap();
        self.effect(move || service.execute(command, &cwd, &CancellationToken::new()))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.base).unwrap();
    }
}

#[test]
fn environment_helper_freezes_only_injected_values_without_global_mutation() {
    let fixture = Fixture::new();
    let mut values = fixture.values();
    values.push(("UNRELATED_SECRET".into(), "never-selected".into()));
    let selected = environment(&values);
    assert_eq!(
        selected,
        NativeEnvironment::new(
            Some(fixture.base.join("config").into_os_string()),
            Some(fixture.base.join("state").into_os_string()),
            Some(fixture.base.join("home").into_os_string()),
        )
    );
    values[2].1 = "changed-only-in-caller-vector".into();
    assert_ne!(environment(&values), selected);
    assert_eq!(environment(&[]), NativeEnvironment::new(None, None, None));
    let invalid = OsString::from_vec(vec![b'/', 0xff]);
    assert_eq!(
        environment(&[("HOME".into(), invalid.clone())]),
        NativeEnvironment::new(None, None, Some(invalid))
    );
}

#[test]
fn interactive_startup_preserves_discovery_order_and_materializes_initial_selections() {
    let fixture = Fixture::new();
    let roots = fixture.roots();
    let expected = fixture.skill("home/project/workspace/skills/local", "local");
    fixture.skill("home/project/.agents/skills/ancestor", "ancestor");
    fixture.skill("state/machine-god/skills/managed", "managed");
    fixture.skill("home/.fx/skills/global", "global");
    let prepared = prepare(
        &fixture.runtime,
        roots,
        environment(&fixture.values()),
        Fixture::terminal(fixture.values()),
        true,
    )
    .unwrap();
    let snapshot = prepared
        .snapshot
        .as_ref()
        .expect("interactive initial snapshot");
    assert!(snapshot.complete());
    assert!(snapshot.diagnostics().is_empty());
    assert_eq!(
        snapshot
            .entries()
            .iter()
            .map(|entry| (entry.metadata.name.as_str(), entry.source()))
            .collect::<Vec<_>>(),
        vec![
            ("local", NativeSkillSource::WorkspaceShared),
            ("ancestor", NativeSkillSource::WorkspaceAgents),
            ("managed", NativeSkillSource::Managed),
            ("global", NativeSkillSource::GlobalFx),
        ]
    );
    let selected = snapshot.resolve("local", None).unwrap();
    let plan = NativeSkillInvocationPlan::resolve("$local review this", snapshot, &[]).unwrap();
    assert_eq!(plan.selections(), std::slice::from_ref(&selected));
    let catalog = Arc::clone(prepared.service.catalog());
    let materialized = fixture
        .effect(move || catalog.materialize(&selected, &CancellationToken::new()))
        .unwrap();
    assert_eq!(materialized.text, expected);
}

#[test]
fn interactive_incomplete_discovery_preserves_diagnostics_and_exact_selection() {
    let fixture = Fixture::new();
    let expected = fixture.skill("home/project/workspace/skills/good", "good");
    fixture.write(
        "home/project/workspace/skills/broken/SKILL.md",
        "---\ndescription: missing required name\n---\nbody",
    );
    let prepared = fixture.prepare(true);
    let snapshot = prepared.snapshot.as_ref().unwrap();
    assert!(!snapshot.complete());
    assert!(!snapshot.diagnostics().is_empty());
    assert_eq!(
        snapshot.resolve("good", None),
        Err(NativeSkillCatalogError::IncompleteDiscovery)
    );
    let plan = NativeSkillInvocationPlan::resolve("$good do work", snapshot, &[]).unwrap();
    assert!(plan.automatic_matching_incomplete());
    assert!(plan.selections().is_empty());
    let selected = snapshot
        .resolve(
            "good",
            Some(&fixture.base.join("home/project/workspace/skills/good")),
        )
        .unwrap();
    let catalog = Arc::clone(prepared.service.catalog());
    let materialized = fixture
        .effect(move || catalog.materialize(&selected, &CancellationToken::new()))
        .unwrap();
    assert_eq!(materialized.text, expected);
}

#[test]
fn one_shot_startup_has_no_snapshot_and_leaves_discovery_explicit() {
    let fixture = Fixture::new();
    fixture.write(
        "home/project/workspace/skills/broken/SKILL.md",
        "---\ndescription: missing required name\n---\nbody",
    );
    let prepared = fixture.prepare(false);
    assert!(prepared.snapshot.is_none());
    assert!(!fixture.base.join("state/machine-god/skills").exists());
    let NativeSkillsServiceResult::Catalog(view) = fixture.execute(&prepared, "list").unwrap()
    else {
        panic!("catalog expected")
    };
    assert!(!view.snapshot.complete());
    assert!(!view.snapshot.diagnostics().is_empty());
}

#[test]
fn initial_selection_cannot_rematch_changed_contents() {
    let fixture = Fixture::new();
    fixture.skill("home/project/workspace/skills/proof", "proof");
    let prepared = fixture.prepare(true);
    let selected = prepared
        .snapshot
        .as_ref()
        .unwrap()
        .resolve("proof", None)
        .unwrap();
    fixture.write(
        "home/project/workspace/skills/proof/SKILL.md",
        "a changed body with a different revision",
    );
    let catalog = Arc::clone(prepared.service.catalog());
    assert_eq!(
        fixture
            .effect(move || catalog.materialize(&selected, &CancellationToken::new()))
            .unwrap_err(),
        NativeSkillCatalogError::StaleSelection
    );
}

#[test]
fn retained_state_and_missing_git_keep_exact_local_create_remove_authority() {
    let fixture = Fixture::new();
    let roots = fixture.roots();
    let original = fixture.base.join("state/machine-god");
    let retained = fixture.base.join("retained-state");
    fs::rename(&original, &retained).unwrap();
    fs::create_dir(&original).unwrap();
    let prepared = prepare(
        &fixture.runtime,
        roots,
        environment(&fixture.values()),
        Fixture::terminal(fixture.values()),
        true,
    )
    .unwrap();
    assert_eq!(prepared.roots.state_root(), original);
    assert!(!fixture.execute(&prepared, "create proof").unwrap().failed());
    assert!(retained.join("skills/proof/SKILL.md").is_file());
    assert!(!original.join("skills").exists());
    assert!(!fixture.execute(&prepared, "remove proof").unwrap().failed());
    assert!(!retained.join("skills/proof").exists());
    assert!(
        matches!(fixture.execute(&prepared, "install https://example.invalid/repository").unwrap_err(), NativeSkillsServiceError::Managed(error) if error.kind == NativeSkillManagedErrorKind::GitUnavailable)
    );
}

#[test]
fn startup_errors_return_without_managed_side_effects_and_runtime_remains_usable() {
    let fixture = Fixture::new();
    for home in [
        "".into(),
        "relative".into(),
        fixture.base.join("missing-home").into_os_string(),
    ] {
        let mut values = fixture.values();
        values[2].1 = home;
        assert!(
            prepare(
                &fixture.runtime,
                fixture.roots(),
                environment(&values),
                Fixture::terminal(values),
                true
            )
            .is_err()
        );
        assert!(!fixture.base.join("state/machine-god/skills").exists());
    }
    let prepared = fixture.prepare(true);
    assert!(prepared.snapshot.as_ref().unwrap().complete());
}

#[test]
fn wrapper_contains_pre_admission_runtime_panic_and_can_start_again() {
    let fixture = Fixture::new();
    let roots = fixture.roots();
    let result = fixture.runtime.block_on(async {
        prepare(
            &fixture.runtime,
            roots,
            environment(&fixture.values()),
            Fixture::terminal(fixture.values()),
            true,
        )
    });
    assert!(result.is_err());
    assert!(!fixture.base.join("state/machine-god/skills").exists());
    assert!(fixture.prepare(false).snapshot.is_none());
}

#[test]
fn wrapper_rejects_replaced_workspace_and_releases_failed_startup_for_retry() {
    let fixture = Fixture::new();
    let roots = fixture.roots();
    let original = fixture.base.join("home/project/workspace");
    let retained = fixture.base.join("home/project/retained-workspace");
    fs::rename(&original, &retained).unwrap();
    fs::create_dir(&original).unwrap();
    assert!(
        prepare(
            &fixture.runtime,
            roots,
            environment(&fixture.values()),
            Fixture::terminal(fixture.values()),
            true
        )
        .is_err()
    );
    assert!(!fixture.base.join("state/machine-god/skills").exists());
    fs::remove_dir(&original).unwrap();
    fs::rename(&retained, &original).unwrap();
    assert!(fixture.prepare(true).snapshot.as_ref().unwrap().complete());
}
