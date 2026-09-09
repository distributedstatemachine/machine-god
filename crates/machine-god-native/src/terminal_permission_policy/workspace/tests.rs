use super::*;
use crate::terminal_permission_policy::tests::Fixture as PolicyFixture;
use crate::workspace_context::{WorkspaceContextRegistration, WorkspaceContextSession};
use crate::{
    NativeSandboxLaunch, NativeSandboxMode, NativeTerminalPermissionPolicy,
    NativeWorkspaceAuthority, NativeWorkspaceContexts, NativeWorkspaceEntrySpec,
    NativeWorkspaceSource, PermissionMode,
};
use machine_god_core::{ToolContext, Turn};
use rustix::fd::OwnedFd;
use rustix::fs::{Mode, OFlags};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    base: PathBuf,
    primary: PathBuf,
    extra: PathBuf,
    authority: NativeWorkspaceAuthority,
    contexts: Arc<NativeWorkspaceContexts>,
    owner: Arc<WorkspaceContextSession>,
}

fn open(path: &Path) -> OwnedFd {
    rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .unwrap()
}

fn spec(path: &Path, saved: bool, launch: bool) -> NativeWorkspaceEntrySpec {
    NativeWorkspaceEntrySpec::new(
        NativeWorkspaceSource::new(path.to_owned(), path.to_owned(), true).unwrap(),
        saved,
        launch,
    )
    .unwrap()
}

impl Fixture {
    fn new(policy: &PolicyFixture) -> Self {
        let base = std::env::temp_dir().join(format!(
            "mg-workspace-sandbox-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&base).unwrap();
        let base = base.canonicalize().unwrap();
        let primary = base.join("primary");
        let extra = base.join("extra");
        let state = base.join("state");
        let suppressed = base.join("suppressed");
        for path in [&primary, &extra, &state, &suppressed] {
            std::fs::create_dir(path).unwrap();
        }
        let nested = primary.join("nested");
        std::fs::create_dir(&nested).unwrap();
        let authority = NativeWorkspaceAuthority::open_blocking(
            open(&primary),
            primary.clone(),
            Some(open(&state)),
            state,
            vec![
                spec(&nested, false, true),
                spec(&extra, false, true),
                spec(&suppressed, true, false),
                spec(&base.join("missing"), false, true),
            ],
            true,
        )
        .unwrap();
        let contexts = Arc::new(NativeWorkspaceContexts::new());
        let owner = contexts.register(&policy.session).unwrap();
        Self {
            base,
            primary,
            extra,
            authority,
            contexts,
            owner,
        }
    }

    fn begin(&self, turn: &Turn) -> WorkspaceContextRegistration {
        self.owner
            .begin(turn, self.authority.snapshot().unwrap())
            .unwrap()
    }

    fn policy(&self, owner: &PolicyFixture) -> NativeTerminalPermissionPolicy {
        let executable = if cfg!(target_os = "macos") {
            Some(File::open(crate::NATIVE_SANDBOX_EXECUTABLE).unwrap())
        } else {
            None
        };
        // Constructor roots deliberately differ; contextual capture may never use them.
        let policy = NativeTerminalPermissionPolicy::new(
            vec![
                NativeSandboxRoot::new(File::open(&self.base).unwrap(), self.base.clone()).unwrap(),
            ],
            executable,
        )
        .unwrap()
        .with_workspace_contexts(self.contexts.clone());
        policy.bind_controller(&owner.controller).unwrap();
        policy
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.base).unwrap();
    }
}

fn capture(
    policy: &NativeTerminalPermissionPolicy,
    context: &ToolContext,
) -> Result<Arc<NativeSandboxLaunch>, NativeSandboxError> {
    policy.capture_on_worker(
        context,
        Instant::now() + Duration::from_secs(5),
        &CancellationToken::new(),
    )
}

#[test]
fn captured_scope_roots_include_only_active_identities_and_nested_roots_deduplicate() {
    let owner = PolicyFixture::new(PermissionMode::Auto, NativeSandboxMode::Os);
    let fixture = Fixture::new(&owner);
    let (turn, _policy_registration, context) = owner.turn();
    let _workspace_registration = fixture.begin(&turn);
    let scope = fixture.contexts.snapshot_for_tool(&context).unwrap();
    fixture
        .authority
        .install(fixture.authority.prepare_blocking(vec![], false).unwrap())
        .unwrap();
    let captured = roots(
        &scope,
        Instant::now() + Duration::from_secs(5),
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(
        captured
            .iter()
            .map(NativeSandboxRoot::canonical_path)
            .collect::<Vec<_>>(),
        vec![fixture.primary.as_path(), fixture.extra.as_path()]
    );
    let policy = fixture.policy(&owner);
    #[cfg(target_os = "macos")]
    {
        let launch = capture(&policy, &context).unwrap();
        assert_eq!(launch.roots().len(), 2);
        assert_eq!(launch.roots()[1].canonical_path(), fixture.extra);
    }
    #[cfg(target_os = "linux")]
    assert_eq!(
        capture(&policy, &context).unwrap_err(),
        NativeSandboxError::Unsupported
    );
}

#[test]
fn none_and_yolo_require_live_scope_without_os_roots_or_launcher() {
    for (mode, configured) in [
        (PermissionMode::Ask, NativeSandboxMode::None),
        (PermissionMode::Yolo, NativeSandboxMode::Os),
    ] {
        let owner = PolicyFixture::new(mode, configured);
        let fixture = Fixture::new(&owner);
        let (turn, _permission, context) = owner.turn();
        let policy = NativeTerminalPermissionPolicy::new(vec![], None)
            .unwrap()
            .with_workspace_contexts(fixture.contexts.clone());
        policy.bind_controller(&owner.controller).unwrap();
        assert_eq!(
            capture(&policy, &context).unwrap_err(),
            NativeSandboxError::Unavailable
        );
        let registration = fixture.begin(&turn);
        let launch = capture(&policy, &context).unwrap();
        assert_eq!(launch.configured(), configured);
        assert_eq!(launch.effective(), NativeSandboxMode::None);
        assert!(launch.roots().is_empty());
        drop(registration);
        assert_eq!(
            launch
                .revalidate(
                    Instant::now() + Duration::from_secs(5),
                    &CancellationToken::new()
                )
                .unwrap_err(),
            NativeSandboxError::Unavailable
        );
        assert_eq!(
            capture(&policy, &context).unwrap_err(),
            NativeSandboxError::Unavailable
        );
    }
}

#[test]
fn cancelled_expired_foreign_and_retired_scopes_never_use_constructor_roots() {
    let owner = PolicyFixture::new(PermissionMode::Ask, NativeSandboxMode::None);
    let fixture = Fixture::new(&owner);
    let (turn, _permission, context) = owner.turn();
    let _registration = fixture.begin(&turn);
    let policy = fixture.policy(&owner);
    let launch = capture(&policy, &context).unwrap();
    let mut foreign = context.clone();
    foreign.session_incarnation_id =
        machine_god_core::SessionIncarnationId::new("foreign").unwrap();
    assert!(capture(&policy, &foreign).is_err());
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert_eq!(
        policy
            .capture_on_worker(
                &context,
                Instant::now() + Duration::from_secs(5),
                &cancelled
            )
            .unwrap_err(),
        NativeSandboxError::Cancelled
    );
    assert_eq!(
        policy
            .capture_on_worker(&context, Instant::now(), &CancellationToken::new())
            .unwrap_err(),
        NativeSandboxError::Timeout
    );
    fixture.owner.retire();
    assert_eq!(
        launch
            .revalidate(
                Instant::now() + Duration::from_secs(5),
                &CancellationToken::new()
            )
            .unwrap_err(),
        NativeSandboxError::Unavailable
    );
}

#[test]
fn maximum_active_root_set_retains_seventeen_and_does_not_multiply_bound() {
    let owner = PolicyFixture::new(PermissionMode::Ask, NativeSandboxMode::None);
    let fixture = Fixture::new(&owner);
    let mut specs = Vec::new();
    for index in 0..16 {
        let path = fixture.base.join(format!("root-{index}"));
        std::fs::create_dir(&path).unwrap();
        specs.push(spec(&path, false, true));
    }
    fixture
        .authority
        .install(fixture.authority.prepare_blocking(specs, false).unwrap())
        .unwrap();
    let (turn, _permission, context) = owner.turn();
    let _registration = fixture.begin(&turn);
    let scope = fixture.contexts.snapshot_for_tool(&context).unwrap();
    let roots = roots(
        &scope,
        Instant::now() + Duration::from_secs(5),
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(roots.len(), MAX_NATIVE_SANDBOX_ROOTS);
}

#[cfg(target_os = "macos")]
#[test]
fn os_roots_revalidate_retained_identity_instead_of_adopting_path_replacement() {
    let owner = PolicyFixture::new(PermissionMode::Auto, NativeSandboxMode::Os);
    let fixture = Fixture::new(&owner);
    let (turn, _permission, context) = owner.turn();
    let _registration = fixture.begin(&turn);
    let launch = capture(&fixture.policy(&owner), &context).unwrap();
    std::fs::rename(&fixture.extra, fixture.base.join("old-extra")).unwrap();
    std::fs::create_dir(&fixture.extra).unwrap();
    assert_eq!(
        launch
            .revalidate(
                Instant::now() + Duration::from_secs(5),
                &CancellationToken::new()
            )
            .unwrap_err(),
        NativeSandboxError::Changed
    );
}

#[cfg(target_os = "linux")]
#[test]
fn none_does_not_require_utf8_profile_representation_for_an_active_root() {
    use std::os::unix::ffi::OsStringExt;
    let owner = PolicyFixture::new(PermissionMode::Yolo, NativeSandboxMode::Os);
    let fixture = Fixture::new(&owner);
    let path = fixture
        .base
        .join(std::ffi::OsString::from_vec(b"raw-\xff".to_vec()));
    std::fs::create_dir(&path).unwrap();
    fixture
        .authority
        .install(
            fixture
                .authority
                .prepare_blocking(vec![spec(&path, false, true)], false)
                .unwrap(),
        )
        .unwrap();
    let (turn, _permission, context) = owner.turn();
    let _registration = fixture.begin(&turn);
    let launch = capture(&fixture.policy(&owner), &context).unwrap();
    assert_eq!(launch.effective(), NativeSandboxMode::None);
    assert!(launch.roots().is_empty());
    let scope = fixture.contexts.snapshot_for_tool(&context).unwrap();
    assert_eq!(
        roots(
            &scope,
            Instant::now() + Duration::from_secs(5),
            &CancellationToken::new()
        )
        .unwrap_err(),
        NativeSandboxError::Invalid
    );
}

#[cfg(target_os = "macos")]
fn pty_helper() -> crate::terminal_pty::TerminalPtyHelper {
    let (program, arguments) = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY").map_or_else(
        || {
            (
                std::env::current_exe().unwrap(),
                vec![
                    "--exact".into(),
                    "terminal_pty::tests::helper_entry".into(),
                    "--nocapture".into(),
                    "--quiet".into(),
                ],
            )
        },
        |program| {
            (
                PathBuf::from(program),
                vec![crate::TERMINAL_PTY_HELPER_ARGUMENT.into()],
            )
        },
    );
    crate::terminal_pty::TerminalPtyHelper::new(program, arguments)
        .unwrap()
        .with_test_inventory_helper()
}

#[cfg(target_os = "macos")]
fn pty_request(
    fixture: &Fixture,
    launch: Arc<NativeSandboxLaunch>,
    command: &str,
) -> crate::terminal_pty::TerminalPtyRequest {
    crate::terminal_pty::TerminalPtyRequest::new(
        "/bin/sh".into(),
        vec!["-c".into(), command.into()],
        vec![("PATH".into(), "/usr/bin:/bin".into())],
        open(&fixture.primary),
        crate::terminal_pty::TerminalPtyDimensions {
            rows: 24,
            columns: 80,
        },
    )
    .unwrap()
    .with_sandbox(launch)
}

#[cfg(target_os = "macos")]
#[test]
fn workspace_scope_is_rechecked_at_actual_pty_release_for_none_and_os() {
    let _serial = crate::os_sandbox::NATIVE_TESTS.lock().unwrap();
    for mode in [NativeSandboxMode::None, NativeSandboxMode::Os] {
        let owner = PolicyFixture::new(PermissionMode::Ask, mode);
        let fixture = Fixture::new(&owner);
        let (turn, _permission, context) = owner.turn();
        let registration = fixture.begin(&turn);
        let launch = capture(&fixture.policy(&owner), &context).unwrap();
        let prepared = crate::terminal_pty::PreparedTerminalPty::prepare_until(
            &pty_helper(),
            pty_request(&fixture, launch, "printf forbidden > forbidden"),
            Instant::now() + Duration::from_secs(10),
            &CancellationToken::new(),
        )
        .unwrap();
        drop(registration);
        assert!(prepared.commit(&CancellationToken::new()).is_err());
        assert!(!fixture.primary.join("forbidden").exists());
    }
}

#[cfg(target_os = "macos")]
#[test]
fn committed_pty_keeps_running_after_source_workspace_turn_ends() {
    let _serial = crate::os_sandbox::NATIVE_TESTS.lock().unwrap();
    let owner = PolicyFixture::new(PermissionMode::Ask, NativeSandboxMode::Os);
    let fixture = Fixture::new(&owner);
    let (turn, _permission, context) = owner.turn();
    let registration = fixture.begin(&turn);
    let launch = capture(&fixture.policy(&owner), &context).unwrap();
    let prepared = crate::terminal_pty::PreparedTerminalPty::prepare_until(
        &pty_helper(), pty_request(&fixture, launch.clone(), "while [ ! -f go ]; do /bin/sleep 0.01; done; printf survived > after-turn; exec /bin/sleep 30"),
        Instant::now()+Duration::from_secs(10), &CancellationToken::new(),
    ).unwrap();
    let mut pty = prepared.commit(&CancellationToken::new()).unwrap();
    drop(registration);
    assert!(
        launch
            .revalidate(
                Instant::now() + Duration::from_secs(5),
                &CancellationToken::new()
            )
            .is_err()
    );
    std::fs::write(fixture.primary.join("go"), "go").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fixture.primary.join("after-turn").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read(fixture.primary.join("after-turn")).unwrap(),
        b"survived"
    );
    pty.close(true).unwrap();
}
