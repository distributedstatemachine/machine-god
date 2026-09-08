use super::*;
use machine_god_native::{NativeSessionCatalogCursor, NativeSessionCatalogInvalidRecords};

fn reporting(limit: usize) -> NativeSessionCatalogQuery {
    NativeSessionCatalogQuery::new(limit)
        .unwrap()
        .with_invalid_records(NativeSessionCatalogInvalidRecords::SkipAndReport)
}

#[test]
fn cursor_pages_ties_then_unknown_activity_to_an_explicit_end() {
    let fixture = Fixture::new();
    for (name, time) in [
        ("z", Some(7)),
        ("a", Some(7)),
        ("old:z", None),
        ("old:a", None),
    ] {
        fixture.save(record(name, time, "/workspace"));
    }
    let mut query = NativeSessionCatalogQuery::new(1).unwrap();
    let mut seen = Vec::new();
    loop {
        let page = fixture.list(query.clone());
        assert!(page.scan_complete());
        assert_eq!(page.entries().len(), 1);
        seen.push(page.entries()[0].id().as_str().to_owned());
        let Some(cursor) = page.next_cursor() else {
            break;
        };
        query = query
            .with_continuation(NativeSessionCatalogCursor::parse(&cursor.to_string()).unwrap());
    }
    assert_eq!(seen, ["z", "a", "old:z", "old:a"]);
    let end =
        fixture.list(query.with_continuation(NativeSessionCatalogCursor::new(None, id("old:a"))));
    assert!(end.entries().is_empty());
    assert_eq!(end.next_cursor(), None);
}

#[test]
fn continuation_reapplies_filters_and_observes_new_writes_without_a_snapshot_claim() {
    let fixture = Fixture::new();
    for (name, time, workspace) in [
        ("match-a", 5, "/workspace"),
        ("match-b", 4, "/workspace"),
        ("match-other", 3, "/other"),
        ("excluded", 2, "/workspace"),
    ] {
        fixture.save(record(name, Some(time), workspace));
    }
    let query = NativeSessionCatalogQuery::new(1)
        .unwrap()
        .with_workspace(Path::new("/workspace"))
        .unwrap();
    let first = fixture.list(query.clone());
    let cursor = first.next_cursor().unwrap();
    fixture.save(record("match-newer", Some(9), "/workspace"));
    let second = fixture.list(query.with_continuation(cursor).with_updated_since(3));
    assert_eq!(second.entries()[0].id(), &id("match-b"));
    assert_eq!(second.matched_count(), 1);
    assert!(second.next_cursor().is_none());
}

#[test]
fn reporting_counts_every_corrupt_candidate_without_repairs_or_identifier_disclosure() {
    let fixture = Fixture::new();
    fixture.save(record("valid-a", Some(5), "/workspace"));
    fixture.save(record("valid-b", Some(4), "/workspace"));
    fs::write(
        fixture.root.join(data_name("malformed-private")),
        b"private malformed",
    )
    .unwrap();
    fs::create_dir(fixture.root.join(data_name("directory"))).unwrap();
    std::os::unix::fs::symlink("missing-target", fixture.root.join(data_name("symlink"))).unwrap();
    let oversized = fs::File::create(fixture.root.join(data_name("oversized"))).unwrap();
    oversized
        .set_len(u64::try_from(MAX_FILE_SESSION_BYTES + 1).unwrap())
        .unwrap();
    let mut bad = record("bad-metadata", Some(2), "/excluded");
    bad.metadata.insert(
        NATIVE_SESSION_METADATA_KEY.into(),
        json!({"schema_version": 99}),
    );
    fixture.save(bad);
    fixture.save(record("bad-lock", Some(1), "/workspace"));
    let lock_path = fixture
        .root
        .join(data_name("bad-lock").replace(".json", ".lock"));
    fs::remove_file(&lock_path).unwrap();
    std::os::unix::fs::symlink("missing-lock-target", &lock_path).unwrap();
    let wrong_id_path = fixture.root.join(data_name("wrong-id"));
    fs::copy(fixture.root.join(data_name("valid-a")), &wrong_id_path).unwrap();
    let source = fs::read(fixture.root.join(data_name("valid-a"))).unwrap();
    assert_eq!(
        block_on(fixture.catalog().list(NativeSessionCatalogQuery::default()))
            .unwrap_err()
            .kind(),
        NativeSessionCatalogErrorKind::Corrupt
    );
    assert!(block_on(list_native_sessions(fixture.environment())).is_err());
    let page = fixture.list(
        reporting(1)
            .with_workspace(Path::new("/workspace"))
            .unwrap(),
    );
    assert!(page.scan_complete());
    assert_eq!(page.skipped_invalid(), 7);
    assert_eq!(page.matched_count(), 2);
    assert!(page.next_cursor().is_some());
    assert_eq!(
        page.latest(),
        Err(NativeSessionSelectionIncomplete::SkippedInvalid)
    );
    assert!(!format!("{page:?}").contains("private"));
    assert_eq!(
        fs::read(fixture.root.join(data_name("malformed-private"))).unwrap(),
        b"private malformed"
    );
    assert_eq!(fs::read(&wrong_id_path).unwrap(), source);
    assert!(
        fs::symlink_metadata(&lock_path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        oversized.metadata().unwrap().len(),
        u64::try_from(MAX_FILE_SESSION_BYTES + 1).unwrap()
    );
}

#[test]
fn all_invalid_is_reported_and_neither_empty_nor_incomplete_pages_fabricate_cursors() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join(data_name("bad")), b"{}").unwrap();
    let page = fixture.list(reporting(1));
    assert!(page.entries().is_empty());
    assert_eq!(page.skipped_invalid(), 1);
    assert_eq!(
        page.latest(),
        Err(NativeSessionSelectionIncomplete::SkippedInvalid)
    );
    assert_eq!(page.next_cursor(), None);
    for i in 0..=MAX_LIST_SESSION_DIRECTORY_ENTRIES {
        fs::write(fixture.root.join(format!("ignored-{i}")), []).unwrap();
    }
    let page = fixture.list(reporting(1));
    assert!(!page.scan_complete());
    assert_eq!(page.next_cursor(), None);
    assert_eq!(
        page.latest(),
        Err(NativeSessionSelectionIncomplete::ScanIncomplete)
    );
}

#[test]
fn rejected_decode_bytes_are_charged_before_next_candidate() {
    let fixture = Fixture::new();
    let bytes = vec![b'x'; 8 * 1024 * 1024];
    for i in 0..9 {
        fs::write(fixture.root.join(data_name(&format!("bad-{i}"))), &bytes).unwrap();
    }
    let page = fixture.list(reporting(1));
    assert!(!page.scan_complete());
    assert_eq!(
        page.scanned_record_bytes(),
        MAX_LIST_SESSION_TOTAL_RECORD_BYTES
    );
    assert_eq!(page.skipped_invalid(), 8);
    assert_eq!(page.scanned_records(), 0);
    assert_eq!(page.next_cursor(), None);
    assert_eq!(
        fs::read(fixture.root.join(data_name("bad-8"))).unwrap(),
        bytes
    );
}

#[test]
fn history_counts_user_groups_not_assistant_and_tool_continuation_messages() {
    let fixture = Fixture::new();
    let mut saved = record("rounds", Some(1), "/workspace");
    saved.messages = [
        Role::User,
        Role::Assistant,
        Role::Tool,
        Role::Assistant,
        Role::User,
        Role::Assistant,
    ]
    .into_iter()
    .map(|role| Message::text(role, "actual canonical message"))
    .collect();
    fixture.save(saved);
    let entry = block_on(fixture.catalog().exact(id("rounds")))
        .unwrap()
        .unwrap();
    assert_eq!(entry.message_count(), 6);
    assert_eq!(entry.history_len(), 2);
}

#[test]
fn current_workspace_facade_is_poll_owned_and_all_does_not_resolve_cwd() {
    use machine_god_native::{
        list_process_current_workspace_session_catalog, list_process_session_catalog,
    };
    const CHILD: &str = "MACHINE_GOD_CATALOG_WORKSPACE_CHILD";
    if let Ok(mode) = std::env::var(CHILD) {
        let base = std::env::current_dir().unwrap();
        if mode == "polled" {
            let future = list_process_current_workspace_session_catalog(reporting(10));
            std::env::set_current_dir(base.join("machine-god")).unwrap();
            let page = block_on(future).unwrap();
            assert_eq!(page.entries().len(), 1);
            assert_eq!(page.entries()[0].id(), &id("root-workspace"));
        } else {
            let removed = base.join("removed-cwd");
            fs::create_dir(&removed).unwrap();
            std::env::set_current_dir(&removed).unwrap();
            fs::remove_dir(&removed).unwrap();
            drop(list_process_current_workspace_session_catalog(reporting(
                10,
            )));
            assert_eq!(
                block_on(list_process_current_workspace_session_catalog(reporting(
                    10
                )))
                .unwrap_err()
                .kind(),
                NativeSessionCatalogErrorKind::Unavailable
            );
            assert_eq!(
                block_on(list_process_session_catalog(reporting(10)))
                    .unwrap()
                    .entries()
                    .len(),
                2
            );
            std::env::set_current_dir(base).unwrap();
        }
        return;
    }
    let fixture = Fixture::new();
    fixture.save(record(
        "base-workspace",
        Some(2),
        fs::canonicalize(&fixture.base).unwrap().to_str().unwrap(),
    ));
    fixture.save(record(
        "root-workspace",
        Some(1),
        fs::canonicalize(&fixture.root).unwrap().to_str().unwrap(),
    ));
    for mode in ["polled", "removed"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "paging::current_workspace_facade_is_poll_owned_and_all_does_not_resolve_cwd",
                "--nocapture",
            ])
            .env(CHILD, mode)
            .env("XDG_STATE_HOME", &fixture.base)
            .env_remove("HOME")
            .current_dir(&fixture.base)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn reporting_does_not_skip_root_or_ordinary_io_failures() {
    let fixture = Fixture::new();
    fixture.save(record("unreadable", Some(1), "/workspace"));
    let path = fixture.root.join(data_name("unreadable"));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o0)).unwrap();
    if fs::File::open(&path).is_err() {
        assert_eq!(
            block_on(fixture.catalog().list(reporting(1)))
                .unwrap_err()
                .kind(),
            NativeSessionCatalogErrorKind::Unavailable
        );
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let wrong_root = fixture.base.join("wrong-root");
    fs::write(&wrong_root, []).unwrap();
    let env = NativeEnvironment::new(None, Some(wrong_root.into_os_string()), None);
    assert_eq!(
        block_on(list_native_session_catalog(env, reporting(1)))
            .unwrap_err()
            .kind(),
        NativeSessionCatalogErrorKind::UnsafeStateRoot
    );
}
