use super::*;
use crate::{NativeManagedSkills, NativeSkillInstallSource, NativeSkillReplacementConsent};
use std::os::unix::fs::PermissionsExt;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "mg-git-runner-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("state")).unwrap();
        Self(root)
    }
    fn runner(&self, script: &str) -> SystemNativeSkillGitRunner {
        let program = self.0.join("git-fixture");
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let (helper, arguments) = match std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY") {
            Some(path) => (
                PathBuf::from(path),
                vec![crate::TERMINAL_CAPTURED_HELPER_ARGUMENT.into()],
            ),
            None => (
                std::env::current_exe().unwrap(),
                vec![
                    "--exact".into(),
                    "terminal_captured_exec::tests::captured_helper_child".into(),
                    "--ignored".into(),
                    "--nocapture".into(),
                    "--quiet".into(),
                ],
            ),
        };
        let mut runner = SystemNativeSkillGitRunner::new(
            program,
            helper,
            arguments,
            vec![
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("TMPDIR".into(), self.0.as_os_str().to_owned()),
            ],
        )
        .unwrap();
        runner.helper = runner.helper.with_test_inventory_helper();
        runner
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn production_git_adapter_composes_direct_argv_retained_cwd_and_managed_publication() {
    let fixture = Fixture::new();
    let runner = fixture
        .runner("#!/bin/sh\nmkdir proof || exit 1\nprintf '%s\\n' \"$@\" > proof/SKILL.md\n");
    let owner =
        NativeManagedSkills::open(&fixture.0.join("state"), Some(Arc::new(runner))).unwrap();
    let source =
        NativeSkillInstallSource::parse("https://example.invalid/repo;literal", None).unwrap();
    let token = CancellationToken::new();
    let plan = owner.prepare_install(&source, &fixture.0, &token).unwrap();
    assert_eq!(plan.items().len(), 1);
    owner
        .commit(plan, &NativeSkillReplacementConsent::NoReplace, &token)
        .unwrap();
    let content = std::fs::read_to_string(fixture.0.join("state/skills/proof/SKILL.md")).unwrap();
    assert!(content.ends_with("--\nhttps://example.invalid/repo;literal\n.\n"));
    assert!(content.contains("protocol.allow=never\n"));
}

#[derive(Debug)]
struct LimitedRunner(SystemNativeSkillGitRunner, Duration, usize);
impl NativeSkillGitRunner for LimitedRunner {
    fn clone_repository(
        &self,
        mut request: NativeSkillGitRequest,
        token: &CancellationToken,
    ) -> Result<(), NativeSkillManagedError> {
        request.deadline = request.deadline.min(Instant::now() + self.1);
        request.max_output_bytes = self.2;
        self.0.clone_repository(request, token)
    }
}

#[test]
fn production_git_adapter_output_limit_and_timeout_clean_owned_groups() {
    for (script, duration, output, expected) in [
        (
            "#!/bin/sh\nprintf '%01000d' 0\n",
            Duration::from_secs(10),
            128,
            Kind::ResourceLimit,
        ),
        (
            "#!/bin/sh\n/bin/sleep 30 &\nwait\n",
            Duration::from_millis(1500),
            65536,
            Kind::TimedOut,
        ),
    ] {
        let fixture = Fixture::new();
        let runner = LimitedRunner(fixture.runner(script), duration, output);
        let owner =
            NativeManagedSkills::open(&fixture.0.join("state"), Some(Arc::new(runner))).unwrap();
        let source = NativeSkillInstallSource::parse("https://example.invalid/repo", None).unwrap();
        let error = owner
            .prepare_install(&source, &fixture.0, &CancellationToken::new())
            .unwrap_err();
        assert_eq!(error.kind, expected);
        assert!(
            error.recovery_id.is_none(),
            "successful cleanup leaves no recovery directory"
        );
        assert_eq!(
            std::fs::read_dir(fixture.0.join("state")).unwrap().count(),
            0
        );
    }
}

#[test]
fn production_git_adapter_cancellation_after_exec_reaps_before_lease_release() {
    let fixture = Fixture::new();
    let runner =
        fixture.runner("#!/bin/sh\nprintf ready > \"$TMPDIR/started\"\n/bin/sleep 30 &\nwait\n");
    let owner =
        NativeManagedSkills::open(&fixture.0.join("state"), Some(Arc::new(runner))).unwrap();
    let token = CancellationToken::new();
    let source = NativeSkillInstallSource::parse("https://example.invalid/repo", None).unwrap();
    std::thread::scope(|threads| {
        threads.spawn(|| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !fixture.0.join("started").exists() {
                assert!(Instant::now() < deadline, "Git fixture reaches exec");
                std::thread::sleep(Duration::from_millis(5));
            }
            token.cancel();
        });
        let error = owner
            .prepare_install(&source, &fixture.0, &token)
            .unwrap_err();
        assert_eq!(error.kind, Kind::Cancelled);
        assert!(error.recovery_id.is_none());
    });
    assert_eq!(
        std::fs::read_dir(fixture.0.join("state")).unwrap().count(),
        0
    );
}

#[test]
fn git_source_and_argv_never_admit_options_local_fallback_or_ext_helpers() {
    for source in [
        "",
        "-u",
        "file:///tmp/repo",
        "/tmp/repo",
        "../repo",
        "ext::anything",
        "git://host/repo",
        "https://host/repo\n",
    ] {
        assert!(validate_url(source).is_err(), "{source:?}");
    }
    for source in [
        "https://host/repo",
        "http://host/repo",
        "ssh://git@host/repo",
        "git@host:owner/repo",
        "https://host/repo;literal",
    ] {
        validate_url(source).unwrap();
        let args = clone_arguments(source);
        assert_eq!(&args[args.len() - 3..], ["--", source, "."]);
        assert!(args.iter().any(|arg| arg == "--no-recurse-submodules"));
    }
}

#[test]
fn git_configuration_is_inert_bounded_and_redacted() {
    let create = |env| {
        SystemNativeSkillGitRunner::new(
            "/not-opened/git".into(),
            "/not-opened/helper".into(),
            vec!["helper".into()],
            env,
        )
    };
    let runner = create(vec![("HOME".into(), "/secret-home".into())]).unwrap();
    assert!(!format!("{runner:?}").contains("secret"));
    for key in [
        "GIT_SSH_COMMAND",
        "GIT_CONFIG_COUNT",
        "LD_PRELOAD",
        "DYLD_INSERT_LIBRARIES",
        "BASH_ENV",
        "GIT_EXEC_PATH",
        "GIT_ASKPASS",
    ] {
        assert!(create(vec![(key.into(), "payload".into())]).is_err());
    }
    assert!(
        create(vec![
            ("PATH".into(), "a".into()),
            ("PATH".into(), "b".into())
        ])
        .is_err()
    );
}

#[test]
fn clone_watchdog_is_descriptor_relative_bounded_and_does_not_follow_links() {
    let root = std::env::temp_dir().join(format!(
        "mg-git-watch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), b"12345").unwrap();
    std::os::unix::fs::symlink("/", root.join("outside")).unwrap();
    let directory = Arc::new(std::fs::File::open(&root).unwrap());
    let token = CancellationToken::new();
    let mut watcher = watchdog::CloneWatchdog::new(
        Arc::clone(&directory),
        5,
        Instant::now() + Duration::from_secs(5),
        &token,
    );
    watcher.check(true).unwrap();
    let renamed = root.with_extension("renamed");
    std::fs::rename(&root, &renamed).unwrap();
    std::fs::create_dir(&root).unwrap();
    std::fs::write(renamed.join("file"), b"123456").unwrap();
    assert_eq!(watcher.check(true).unwrap_err().kind, Kind::ResourceLimit);
    token.cancel();
    assert_eq!(watcher.check(true).unwrap_err().kind, Kind::Cancelled);
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::remove_dir_all(&renamed).unwrap();
}
