use super::*;
use machine_god_native::{NativeSessionCatalogCursor, NativeSessionCatalogInvalidRecords};
use std::os::unix::fs::symlink;

#[test]
fn workspace_alias_resolution_is_opt_in_deferred_and_preserves_every_predicate() {
    let fixture = Fixture::new();
    let canonical = fs::canonicalize(&fixture.base).unwrap();
    let workspace = canonical.join("workspace");
    fs::create_dir(&workspace).unwrap();
    let alias = canonical.join("alias");
    // Resolve an ancestor alias, just like an ACP new/load/resume request.
    let requested = alias.join("workspace");
    let query = NativeSessionCatalogQuery::new(1)
        .unwrap()
        .with_workspace(&requested)
        .unwrap()
        .with_search("match")
        .unwrap()
        .with_updated_since(2)
        .with_continuation(NativeSessionCatalogCursor::new(Some(5), id("match-newer")))
        .with_invalid_records(NativeSessionCatalogInvalidRecords::SkipAndReport);
    let future = query.clone().resolve_workspace_alias();
    // A constructor-time resolution would retain the missing alias spelling.
    symlink(&canonical, &alias).unwrap();
    for (name, time, path) in [
        ("match-newer", 5, workspace.as_path()),
        ("match-a", 4, workspace.as_path()),
        ("match-b", 3, workspace.as_path()),
        ("excluded", 3, workspace.as_path()),
        ("match-old", 1, workspace.as_path()),
        ("match-other", 3, canonical.as_path()),
    ] {
        let mut entry = record(name, Some(time), path.to_str().unwrap());
        // Search matches canonical user previews, never session ID spelling.
        entry.messages.push(Message::text(Role::User, name));
        fixture.save(entry);
    }
    fs::write(fixture.root.join(data_name("corrupt")), b"invalid").unwrap();
    assert!(
        fixture.list(query).entries().is_empty(),
        "literal query stays pure"
    );
    let query = block_on(future).unwrap();
    assert_eq!(query.limit(), 1);
    let page = fixture.list(query);
    assert_eq!(page.entries()[0].id(), &id("match-a"));
    assert_eq!(page.entries().len(), 1);
    assert_eq!(page.matched_count(), 2);
    assert_eq!(page.skipped_invalid(), 1);
    assert!(page.next_cursor().is_some());
}

#[test]
fn workspace_alias_resolution_keeps_deleted_history_without_recreating_paths() {
    let fixture = Fixture::new();
    let canonical = fs::canonicalize(&fixture.base).unwrap();
    let workspace = canonical.join("deleted");
    fs::create_dir(&workspace).unwrap();
    fixture.save(record("retained", Some(1), workspace.to_str().unwrap()));
    let before = fs::read(fixture.root.join(data_name("retained"))).unwrap();
    fs::remove_dir(&workspace).unwrap();
    let query = NativeSessionCatalogQuery::default()
        .with_workspace(&workspace)
        .unwrap();
    assert_eq!(
        fixture
            .list(block_on(query.resolve_workspace_alias()).unwrap())
            .entries()[0]
            .id(),
        &id("retained")
    );
    assert!(!workspace.exists());
    // A removed ancestor replaced by a regular file is also absent history,
    // not permission to create a directory or read that file's contents.
    let former_child = workspace.join("child");
    fixture.save(record(
        "retained-child",
        Some(2),
        former_child.to_str().unwrap(),
    ));
    fs::write(&workspace, b"not a workspace").unwrap();
    let query = NativeSessionCatalogQuery::default()
        .with_workspace(&former_child)
        .unwrap();
    assert_eq!(
        fixture
            .list(block_on(query.resolve_workspace_alias()).unwrap())
            .entries()[0]
            .id(),
        &id("retained-child")
    );
    assert_eq!(fs::read(&workspace).unwrap(), b"not a workspace");
    assert_eq!(
        fs::read(fixture.root.join(data_name("retained"))).unwrap(),
        before
    );
}

#[test]
fn workspace_alias_resolution_rejects_nonabsence_errors_without_path_disclosure() {
    let fixture = Fixture::new();
    let looping = fs::canonicalize(&fixture.base)
        .unwrap()
        .join("private-alias-loop");
    symlink(&looping, &looping).unwrap();
    let query = NativeSessionCatalogQuery::default()
        .with_workspace(&looping)
        .unwrap();
    drop(query.clone().resolve_workspace_alias());
    let error = block_on(query.resolve_workspace_alias()).unwrap_err();
    assert_eq!(error.kind(), NativeSessionCatalogErrorKind::Unavailable);
    assert!(!format!("{error:?} {error}").contains("private-alias-loop"));
    // No selected filter means no workspace lookup at all.
    assert_eq!(
        block_on(
            NativeSessionCatalogQuery::new(7)
                .unwrap()
                .resolve_workspace_alias()
        )
        .unwrap()
        .limit(),
        7
    );
}
