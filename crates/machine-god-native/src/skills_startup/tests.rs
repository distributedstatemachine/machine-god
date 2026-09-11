use super::*;
use crate::skills_managed::NativeSkillManagedErrorKind;
use crate::skills_service::{NativeSkillsServiceError, NativeSkillsServiceResult};
use crate::{NativeRootSelection, NativeSkillsCommand};
use futures_executor::block_on;
use std::{
    fs,
    os::unix::{ffi::OsStringExt, fs::PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mg-skills-startup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        let path = fs::canonicalize(path).unwrap();
        for relative in ["home", "home/workspace", "state", "bin"] {
            fs::create_dir(path.join(relative)).unwrap();
            fs::set_permissions(path.join(relative), fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self(path)
    }
    fn environment(&self) -> NativeEnvironment {
        NativeEnvironment::new(
            None,
            Some(self.0.join("state").into_os_string()),
            Some(self.0.join("home").into_os_string()),
        )
    }
    fn roots(&self) -> PreparedNativeRoots {
        PreparedNativeRoots::prepare(
            NativeRootSelection::from_environment(
                &self.environment(),
                &self.0.join("home/workspace"),
            )
            .unwrap(),
        )
        .unwrap()
    }
    fn terminal(&self) -> NativeReferenceHostTerminalOptions {
        NativeReferenceHostTerminalOptions::new(
            self.0.join("never-executed-helper"),
            None,
            vec![("PATH".into(), self.0.join("bin").into_os_string())],
        )
        .unwrap()
    }
    fn startup(&self) -> (PreparedNativeRoots, Arc<NativeSkillsService>) {
        let scope = NativeOwnedWorkerScope::new();
        let result = block_on(prepare_native_skills(
            self.roots(),
            self.environment(),
            self.terminal(),
            scope.clone(),
            CancellationToken::new(),
        ))
        .unwrap();
        settle(&scope);
        result
    }
    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn settle(scope: &NativeOwnedWorkerScope) {
    scope.close();
    let completion = scope.completion();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !completion.is_complete() {
        assert!(
            Instant::now() < deadline,
            "owned startup worker must settle"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn execute(
    service: &NativeSkillsService,
    command: &str,
    cwd: &Path,
) -> Result<NativeSkillsServiceResult, NativeSkillsServiceError> {
    service.execute(
        command.parse::<NativeSkillsCommand>().unwrap(),
        cwd,
        &CancellationToken::new(),
    )
}

#[test]
fn startup_is_inert_until_poll_and_never_discovers_creates_or_launches() {
    let fixture = Fixture::new();
    fixture.write(
        "home/workspace/skills/bad/SKILL.md",
        "---\nname: [bad]\n---\n",
    );
    fixture.write(
        "bin/git",
        "not a real executable: startup must never launch this",
    );
    fs::set_permissions(fixture.0.join("bin/git"), fs::Permissions::from_mode(0o700)).unwrap();
    let scope = NativeOwnedWorkerScope::new();
    let future = prepare_native_skills(
        fixture.roots(),
        fixture.environment(),
        fixture.terminal(),
        scope.clone(),
        CancellationToken::new(),
    );
    scope.close();
    assert!(scope.completion().is_complete());
    drop(future);
    assert!(!fixture.0.join("state/machine-god/skills").exists());
    let (_, service) = fixture.startup();
    assert!(!fixture.0.join("state/machine-god/skills").exists());
    assert!(matches!(
        execute(&service, "path", &fixture.0).unwrap(),
        NativeSkillsServiceResult::Path(_)
    ));
}

#[test]
fn retained_state_rename_preserves_catalog_manager_identity_and_removal() {
    let fixture = Fixture::new();
    let roots = fixture.roots();
    let old_state = fixture.0.join("state/machine-god");
    let renamed = fixture.0.join("retained-state");
    fs::rename(&old_state, &renamed).unwrap();
    fs::create_dir(&old_state).unwrap();
    let scope = NativeOwnedWorkerScope::new();
    let (returned, service) = block_on(prepare_native_skills(
        roots,
        fixture.environment(),
        fixture.terminal(),
        scope.clone(),
        CancellationToken::new(),
    ))
    .unwrap();
    settle(&scope);
    assert_eq!(returned.state_root(), old_state);
    assert!(
        !execute(&service, "create proof", &fixture.0)
            .unwrap()
            .failed()
    );
    assert!(renamed.join("skills/proof/SKILL.md").is_file());
    assert!(!old_state.join("skills").exists());
    assert!(
        !execute(&service, "remove proof", &fixture.0)
            .unwrap()
            .failed()
    );
    assert!(!renamed.join("skills/proof").exists());
}

#[test]
fn captured_canonical_workspace_and_explicit_home_alias_compose_without_reopening_workspace() {
    let fixture = Fixture::new();
    fixture.write("home/workspace/skills/proof/SKILL.md", "proof");
    let alias = fixture.0.join("home-alias");
    std::os::unix::fs::symlink(fixture.0.join("home"), &alias).unwrap();
    let environment = NativeEnvironment::new(
        None,
        Some(fixture.0.join("state").into_os_string()),
        Some(alias.clone().into_os_string()),
    );
    let roots = PreparedNativeRoots::prepare(
        NativeRootSelection::from_environment(&environment, &alias.join("workspace")).unwrap(),
    )
    .unwrap();
    let scope = NativeOwnedWorkerScope::new();
    let (returned, service) = block_on(prepare_native_skills(
        roots,
        environment,
        fixture.terminal(),
        scope.clone(),
        CancellationToken::new(),
    ))
    .unwrap();
    settle(&scope);
    assert_eq!(returned.workspace_root(), alias.join("workspace"));
    let NativeSkillsServiceResult::Catalog(view) = execute(&service, "list", &fixture.0).unwrap()
    else {
        panic!("catalog expected")
    };
    assert_eq!(view.snapshot.entries().len(), 1);
}

#[test]
fn replaced_workspace_fails_ancestry_without_discovering_replacement() {
    let fixture = Fixture::new();
    let roots = fixture.roots();
    fs::rename(
        fixture.0.join("home/workspace"),
        fixture.0.join("home/retained-workspace"),
    )
    .unwrap();
    fixture.write("home/workspace/skills/replacement/SKILL.md", "replacement");
    let scope = NativeOwnedWorkerScope::new();
    let error = block_on(prepare_native_skills(
        roots,
        fixture.environment(),
        fixture.terminal(),
        scope.clone(),
        CancellationToken::new(),
    ))
    .unwrap_err();
    settle(&scope);
    assert_eq!(
        error,
        NativeSkillsStartupError::Roots(NativeSkillRootsError::ChangedAncestry)
    );
    assert!(!fixture.0.join("state/machine-god/skills").exists());
}

#[test]
fn home_missing_is_explicit_but_invalid_supplied_home_never_falls_back() {
    let fixture = Fixture::new();
    let missing =
        NativeEnvironment::new(None, Some(fixture.0.join("state").into_os_string()), None);
    let scope = NativeOwnedWorkerScope::new();
    assert!(
        block_on(prepare_native_skills(
            fixture.roots(),
            missing,
            fixture.terminal(),
            scope.clone(),
            CancellationToken::new()
        ))
        .is_ok()
    );
    settle(&scope);
    for home in [
        "".into(),
        "relative".into(),
        "/does-not-exist-mg-skills-home".into(),
        std::ffi::OsString::from_vec(vec![b'/', 0xff]),
    ] {
        let environment = NativeEnvironment::new(None, None, Some(home));
        let scope = NativeOwnedWorkerScope::new();
        assert_eq!(
            block_on(prepare_native_skills(
                fixture.roots(),
                environment,
                fixture.terminal(),
                scope.clone(),
                CancellationToken::new()
            ))
            .unwrap_err(),
            NativeSkillsStartupError::InvalidHome
        );
        settle(&scope);
    }
}

#[test]
fn missing_git_keeps_local_management_and_returns_typed_remote_unavailability() {
    let fixture = Fixture::new();
    let (_, service) = fixture.startup();
    assert!(
        !execute(&service, "create local", &fixture.0)
            .unwrap()
            .failed()
    );
    let error = execute(
        &service,
        "add https://example.invalid/repository",
        &fixture.0,
    )
    .unwrap_err();
    assert!(
        matches!(error, NativeSkillsServiceError::Managed(error) if error.kind == NativeSkillManagedErrorKind::GitUnavailable)
    );
}

#[test]
fn frozen_path_lookup_bounds_and_environment_consistency_are_explicit() {
    let fixture = Fixture::new();
    let token = CancellationToken::new();
    let mut budget = selection::Budget::new(&token);
    assert!(
        selection::git(&fixture.terminal(), &mut budget)
            .unwrap()
            .is_none()
    );
    fixture.write("bin/git", "must not execute");
    fs::set_permissions(fixture.0.join("bin/git"), fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        selection::git(&fixture.terminal(), &mut budget)
            .unwrap()
            .is_some()
    );
    for path in [
        String::new(),
        ":/bin".into(),
        "/bin:".into(),
        "relative".into(),
        "/bin:relative".into(),
    ] {
        let terminal = NativeReferenceHostTerminalOptions::new(
            fixture.0.join("helper"),
            None,
            vec![("PATH".into(), path.into())],
        )
        .unwrap();
        assert_eq!(
            selection::git(&terminal, &mut budget).unwrap_err(),
            NativeSkillsStartupError::InvalidPath
        );
    }
    let terminal = NativeReferenceHostTerminalOptions::new(
        fixture.0.join("helper"),
        None,
        vec![(
            "PATH".into(),
            "/bin:".repeat(65).trim_end_matches(':').into(),
        )],
    )
    .unwrap();
    assert_eq!(
        selection::git(&terminal, &mut budget).unwrap_err(),
        NativeSkillsStartupError::ResourceLimit
    );
    assert!(
        NativeReferenceHostTerminalOptions::new(
            fixture.0.join("helper"),
            None,
            vec![(
                "PATH".into(),
                format!("/{}", "x".repeat(MAX_NATIVE_SKILLS_STARTUP_PATH_BYTES)).into()
            )],
        )
        .is_err(),
        "the frozen terminal snapshot independently rejects oversized PATH values"
    );
    let terminal = NativeReferenceHostTerminalOptions::new(
        fixture.0.join("helper"),
        None,
        vec![("HOME".into(), fixture.0.join("wrong").into_os_string())],
    )
    .unwrap();
    assert_eq!(
        selection::validate_environment(&fixture.environment(), &terminal),
        Err(NativeSkillsStartupError::InvalidEnvironment)
    );
}

#[test]
fn cancelled_and_closed_startup_never_admit_effects() {
    let fixture = Fixture::new();
    let token = CancellationToken::new();
    token.cancel();
    let scope = NativeOwnedWorkerScope::new();
    assert_eq!(
        block_on(prepare_native_skills(
            fixture.roots(),
            fixture.environment(),
            fixture.terminal(),
            scope.clone(),
            token
        ))
        .unwrap_err(),
        NativeSkillsStartupError::Cancelled
    );
    scope.close();
    assert!(scope.completion().is_complete());
    assert_eq!(
        block_on(prepare_native_skills(
            fixture.roots(),
            fixture.environment(),
            fixture.terminal(),
            scope.clone(),
            CancellationToken::new()
        ))
        .unwrap_err(),
        NativeSkillsStartupError::Admission
    );
}

#[test]
fn dropped_or_cancelled_response_preserves_owned_completion_and_private_token() {
    for abandon in [false, true] {
        let scope = NativeOwnedWorkerScope::new();
        let caller = CancellationToken::new();
        let (started, ready) = mpsc::sync_channel(1);
        let (resume, wait) = mpsc::sync_channel(1);
        let mut future = on_worker(scope.clone(), caller.clone(), move |stop| {
            started.send(stop.clone()).unwrap();
            wait.recv().unwrap();
            selection::Budget::new(&stop).check()
        });
        let mut cx = Context::from_waker(std::task::Waker::noop());
        assert!(matches!(future.as_mut().poll(&mut cx), Poll::Pending));
        let private = ready.recv_timeout(Duration::from_secs(5)).unwrap();
        if abandon {
            drop(future);
            assert!(!caller.is_cancelled());
        } else {
            caller.cancel();
            assert_eq!(block_on(future), Err(NativeSkillsStartupError::Cancelled));
        }
        assert!(private.is_cancelled());
        scope.close();
        assert!(!scope.completion().is_complete());
        resume.send(()).unwrap();
        settle(&scope);
    }
}

#[test]
fn startup_meter_checks_cancellation_after_native_returns() {
    let token = CancellationToken::new();
    let mut budget = selection::Budget::new(&token);
    assert_eq!(
        budget.call(|| {
            token.cancel();
            Ok(())
        }),
        Err(NativeSkillsStartupError::Cancelled)
    );
    let token = CancellationToken::new();
    let mut budget = selection::Budget::new(&token);
    for _ in 0..MAX_NATIVE_SKILLS_STARTUP_IO_ATTEMPTS {
        budget.call(|| Ok(())).unwrap();
    }
    assert_eq!(
        budget.call(|| Ok(())),
        Err(NativeSkillsStartupError::ResourceLimit)
    );
}
