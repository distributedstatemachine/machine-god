use super::*;

#[test]
fn validation_preserves_present_bytes_and_missing_namespaces_without_artifacts() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let absent = store.load().unwrap();
    store.validate_unchanged(&absent).unwrap();
    store.validate_unchanged(&absent).unwrap();
    assert!(!fixture.base.join("missing").exists());

    let original = br#"{ "mcp": {} }"#;
    fixture.seed(original);
    let present = store.load().unwrap();
    store.validate_unchanged(&present).unwrap();
    store.validate_unchanged(&present).unwrap();
    assert_eq!(fs::read(store.path()).unwrap(), original);
    assert_eq!(fs::read_dir(fixture.directory()).unwrap().count(), 1);
    assert_eq!(
        store.validate_unchanged(&absent).unwrap_err(),
        NativeMcpConfigStoreError::Conflict
    );
}

#[test]
fn validation_rejects_foreign_store_and_equal_byte_inode_replacement() {
    let fixture = Fixture::new();
    fixture.seed(b"{}");
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    assert_eq!(
        fixture.store().validate_unchanged(&snapshot).unwrap_err(),
        NativeMcpConfigStoreError::Conflict
    );
    fixture.write("replacement", b"{}");
    fs::rename(fixture.directory().join("replacement"), store.path()).unwrap();
    assert_eq!(
        store.validate_unchanged(&snapshot).unwrap_err(),
        NativeMcpConfigStoreError::Conflict
    );
    assert_eq!(fs::read(store.path()).unwrap(), b"{}");
    assert_eq!(fs::read_dir(fixture.directory()).unwrap().count(), 1);
}

#[test]
fn validation_rejects_changed_and_removed_data() {
    let fixture = Fixture::new();
    fixture.seed(b"{}");
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    fixture.write("mcp.json", br#"{"mcp":{}}"#);
    assert_eq!(
        store.validate_unchanged(&snapshot).unwrap_err(),
        NativeMcpConfigStoreError::Conflict
    );
    let snapshot = store.load().unwrap();
    fs::remove_file(store.path()).unwrap();
    assert_eq!(
        store.validate_unchanged(&snapshot).unwrap_err(),
        NativeMcpConfigStoreError::Conflict
    );
    assert_eq!(fs::read_dir(fixture.directory()).unwrap().count(), 0);
}

#[test]
fn validation_rejects_replaced_root_even_with_matching_data() {
    let fixture = Fixture::new();
    fixture.seed(b"{}");
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    fs::rename(fixture.directory(), fixture.base.join("previous")).unwrap();
    fixture.seed(b"{}");
    assert_eq!(
        store.validate_unchanged(&snapshot).unwrap_err(),
        NativeMcpConfigStoreError::Conflict
    );
    assert_eq!(fs::read_dir(fixture.directory()).unwrap().count(), 1);
    assert_eq!(
        fs::read(fixture.base.join("previous/mcp.json")).unwrap(),
        b"{}"
    );
}

#[test]
fn validation_rejects_replaced_observed_ancestor_before_missing_components() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    let moved = fixture.base.with_extension("moved");
    fs::rename(&fixture.base, &moved).unwrap();
    fs::create_dir(&fixture.base).unwrap();
    let result = store.validate_unchanged(&snapshot);
    assert_eq!(result.unwrap_err(), NativeMcpConfigStoreError::Conflict);
    assert!(!fixture.base.join("missing").exists());
    assert!(!moved.join("missing").exists());
    fs::remove_dir(moved).unwrap();
}

#[test]
fn validation_rejects_unsafe_replacement_without_following_it() {
    let fixture = Fixture::new();
    fixture.seed(b"{}");
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    let outside = fixture.base.join("outside");
    fs::rename(store.path(), &outside).unwrap();
    symlink(&outside, store.path()).unwrap();
    assert_eq!(
        store.validate_unchanged(&snapshot).unwrap_err(),
        NativeMcpConfigStoreError::UnsafePath
    );
    assert!(fs::symlink_metadata(store.path()).unwrap().is_symlink());
    assert_eq!(fs::read(outside).unwrap(), b"{}");
    assert_eq!(fs::read_dir(fixture.directory()).unwrap().count(), 1);
}
