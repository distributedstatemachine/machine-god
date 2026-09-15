use super::*;
use crate::NativeSessionOrigin;

fn prepare(fixture: &Fixture, workspace: &str, origin: NativeSessionOrigin) -> OwnedFd {
    block_on(ManagedJournal::workspace_directory(
        fixture.root(),
        workspace.into(),
        origin,
        fixture.workers.clone(),
    ))
    .unwrap()
}

fn identity(fd: &OwnedFd) -> (i128, i128) {
    let stat = rustix::fs::fstat(fd).unwrap();
    (i128::from(stat.st_dev), i128::from(stat.st_ino))
}

#[test]
fn workspace_namespace_is_inert_and_reuses_exact_workspace_origin() {
    let fixture = Fixture::new();
    let future = ManagedJournal::workspace_directory(
        fixture.root(),
        "/not-opened/workspace".into(),
        NativeSessionOrigin::Cli,
        fixture.workers.clone(),
    );
    assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), 0);
    drop(future);
    assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), 0);
    let first = prepare(&fixture, "/not-opened/workspace", NativeSessionOrigin::Cli);
    let same = prepare(&fixture, "/not-opened/workspace", NativeSessionOrigin::Cli);
    let other = prepare(&fixture, "/not-opened/other", NativeSessionOrigin::Cli);
    let acp = prepare(&fixture, "/not-opened/workspace", NativeSessionOrigin::Acp);
    assert_eq!(identity(&first), identity(&same));
    assert_ne!(identity(&first), identity(&other));
    assert_ne!(identity(&first), identity(&acp));
    assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), 3);
    assert_eq!(rustix::fs::fstat(&first).unwrap().st_mode & 0o777, 0o700);
}

#[test]
fn workspace_namespace_refuses_nonprivate_root_without_creating_entries() {
    let fixture = Fixture::new();
    std::fs::set_permissions(&fixture.path, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        block_on(ManagedJournal::workspace_directory(
            fixture.root(),
            "/workspace".into(),
            NativeSessionOrigin::Cli,
            fixture.workers.clone(),
        )),
        Err(JournalError::Invalid)
    ));
    assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), 0);
}

#[test]
fn workspace_namespace_refuses_symlink_and_nonprivate_existing_directory() {
    let fixture = Fixture::new();
    drop(prepare(&fixture, "/workspace", NativeSessionOrigin::Cli));
    let entry = std::fs::read_dir(&fixture.path)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::set_permissions(&entry, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        block_on(ManagedJournal::workspace_directory(
            fixture.root(),
            "/workspace".into(),
            NativeSessionOrigin::Cli,
            fixture.workers.clone(),
        )),
        Err(JournalError::Invalid)
    ));
    let destination = fixture.path.join("retained-original");
    std::fs::rename(&entry, &destination).unwrap();
    std::os::unix::fs::symlink(&destination, &entry).unwrap();
    assert!(matches!(
        block_on(ManagedJournal::workspace_directory(
            fixture.root(),
            "/workspace".into(),
            NativeSessionOrigin::Cli,
            fixture.workers.clone(),
        )),
        Err(JournalError::Invalid)
    ));
    assert_eq!(std::fs::read_dir(destination).unwrap().count(), 0);
}

#[test]
fn workspace_namespace_preserves_exclusive_journal_custody_until_last_owner_drops() {
    let fixture = Fixture::new();
    let open = || {
        ManagedJournal::open(
            prepare(&fixture, "/workspace", NativeSessionOrigin::Cli),
            fixture.workers.clone(),
            JournalLimits::default(),
        )
    };
    let journal = block_on(open()).unwrap();
    let lease = journal.owner_lease();
    assert!(matches!(block_on(open()), Err(JournalError::Busy)));
    drop(journal);
    assert!(matches!(block_on(open()), Err(JournalError::Busy)));
    drop(lease);
    drop(block_on(open()).unwrap());
}
