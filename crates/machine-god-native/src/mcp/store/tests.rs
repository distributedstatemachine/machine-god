use super::*;
use futures_executor::block_on;
use std::fs;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};
use std::sync::atomic::{AtomicU64, Ordering};

mod validation;

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    base: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "mg-mcp-profile-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&base).unwrap();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            base: fs::canonicalize(base).unwrap(),
        }
    }
    fn directory(&self) -> PathBuf {
        self.base.join("missing/nested/profile")
    }
    fn store(&self) -> NativeMcpConfigStore {
        NativeMcpConfigStore::new(self.directory()).unwrap()
    }
    fn seed(&self, bytes: &[u8]) {
        fs::create_dir_all(self.directory()).unwrap();
        fs::set_permissions(self.directory(), fs::Permissions::from_mode(0o700)).unwrap();
        self.write("mcp.json", bytes);
    }
    fn write(&self, name: &str, bytes: &[u8]) {
        fs::write(self.directory().join(name), bytes).unwrap();
        fs::set_permissions(
            self.directory().join(name),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.base).unwrap();
    }
}
fn server(name: &str, command: &str) -> McpServerConfig {
    McpServerConfig::stdio(name, command, &[]).unwrap()
}
fn insert(name: &str) -> McpConfigMutation {
    McpConfigMutation::Insert(server(name, "node"))
}

#[test]
fn constructor_load_unpolled_future_and_missing_noop_are_inert() {
    let fixture = Fixture::new();
    let store = fixture.store();
    assert_eq!(store.path(), fixture.directory().join("mcp.json"));
    let snapshot = store.load().unwrap();
    assert!(snapshot.config().servers().is_empty());
    let mutation = insert("test");
    drop(store.apply(&snapshot, &mutation));
    let receipt =
        block_on(store.apply(&snapshot, &McpConfigMutation::Remove("unknown".into()))).unwrap();
    assert!(!receipt.changed());
    assert_eq!(receipt.durability(), McpConfigCommitDurability::Confirmed);
    assert!(!fixture.base.join("missing").exists());
}

#[test]
fn first_publication_creates_private_namespace_and_canonical_profile_only() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    let receipt = block_on(store.apply(&snapshot, &insert("first"))).unwrap();
    assert!(receipt.changed());
    assert!(receipt.before().servers().is_empty());
    assert_eq!(receipt.intended().servers()[0].name(), "first");
    assert_eq!(receipt.durability(), McpConfigCommitDurability::Confirmed);
    assert_eq!(store.load().unwrap().config(), receipt.intended());
    assert_eq!(
        fs::read(store.path()).unwrap(),
        receipt.intended().encode().unwrap()
    );
    for path in [
        fixture.base.join("missing"),
        fixture.base.join("missing/nested"),
        fixture.directory(),
    ] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    for name in ["mcp.json", ".mcp.lock"] {
        assert_eq!(
            fs::metadata(fixture.directory().join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert!(!fixture.directory().join("config.json").exists());
    assert!(!fixture.directory().join(".config.lock").exists());
    assert!(!fixture.directory().join(".mcp.tmp").exists());
}

#[test]
fn explicit_mutations_preserve_order_and_existing_equal_noop_bytes() {
    let fixture = Fixture::new();
    let original = br#"{ "mcp": {"z":{"command":["node"]},"a":{"command":"node"}} }"#;
    fixture.seed(original);
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    let same = McpConfigMutation::Replace(server("z", "node"));
    assert!(!block_on(store.apply(&snapshot, &same)).unwrap().changed());
    assert_eq!(fs::read(store.path()).unwrap(), original);
    assert!(!fixture.directory().join(".mcp.lock").exists());
    assert!(matches!(
        block_on(store.apply(&snapshot, &insert("z"))),
        Err(NativeMcpConfigStoreError::InvalidConfig(
            McpConfigError::AlreadyExists
        ))
    ));
    let receipt =
        block_on(store.apply(&snapshot, &McpConfigMutation::Replace(server("z", "new")))).unwrap();
    assert_eq!(
        receipt
            .intended()
            .servers()
            .iter()
            .map(McpServerConfig::name)
            .collect::<Vec<_>>(),
        ["z", "a"]
    );
    let snapshot = store.load().unwrap();
    let receipt = block_on(store.apply(&snapshot, &McpConfigMutation::Remove("z".into()))).unwrap();
    assert_eq!(receipt.intended().servers()[0].name(), "a");
    let snapshot = store.load().unwrap();
    assert!(
        block_on(store.apply(
            &snapshot,
            &McpConfigMutation::Replace(server("new", "node"))
        ))
        .unwrap()
        .changed()
    );
    assert_eq!(store.load().unwrap().config().servers()[1].name(), "new");
}

#[test]
fn foreign_stale_and_equal_byte_replacement_reject_even_noops() {
    let fixture = Fixture::new();
    fixture.seed(b"{}");
    let store = fixture.store();
    let foreign = fixture.store();
    let snapshot = store.load().unwrap();
    let noop = McpConfigMutation::Remove("unknown".into());
    assert_eq!(
        block_on(foreign.apply(&snapshot, &noop)).unwrap_err(),
        NativeMcpConfigStoreError::Conflict
    );
    fixture.write("replacement", b"{}");
    fs::rename(fixture.directory().join("replacement"), store.path()).unwrap();
    assert_eq!(
        block_on(store.apply(&snapshot, &noop)).unwrap_err(),
        NativeMcpConfigStoreError::Conflict
    );
    assert_eq!(
        block_on(store.apply(&snapshot, &insert("new"))).unwrap_err(),
        NativeMcpConfigStoreError::Conflict
    );
    assert_eq!(fs::read(store.path()).unwrap(), b"{}");
    assert!(!fixture.directory().join(".mcp.tmp").exists());
}

#[test]
fn in_place_source_change_rejects_old_snapshot() {
    let fixture = Fixture::new();
    fixture.seed(b"{}");
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    fixture.write("mcp.json", br#"{"mcp":{}}"#);
    assert_eq!(
        block_on(store.apply(&snapshot, &insert("new"))).unwrap_err(),
        NativeMcpConfigStoreError::Conflict
    );
    assert_eq!(fs::read(store.path()).unwrap(), br#"{"mcp":{}}"#);
}

#[test]
fn data_must_be_private_and_singly_linked_while_settings_legacy_remains_readable() {
    let fixture = Fixture::new();
    fixture.seed(b"{}");
    let store = fixture.store();
    fs::set_permissions(store.path(), fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        store.load().unwrap_err(),
        NativeMcpConfigStoreError::UnsafePath
    );
    fs::set_permissions(store.path(), fs::Permissions::from_mode(0o600)).unwrap();
    fs::hard_link(store.path(), fixture.base.join("alias")).unwrap();
    assert_eq!(
        store.load().unwrap_err(),
        NativeMcpConfigStoreError::UnsafePath
    );
    fs::remove_file(fixture.base.join("alias")).unwrap();
    assert!(store.load().is_ok());
    fixture.write(
        "config.json",
        br#"{"schema_version":1,"permission_mode":"ask"}"#,
    );
    fs::set_permissions(
        fixture.directory().join("config.json"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(
        crate::NativeUserConfigStore::new(fixture.directory())
            .load()
            .is_ok()
    );
}

#[test]
fn data_root_and_lock_symlinks_do_not_redirect_authority() {
    let fixture = Fixture::new();
    fixture.seed(b"{}");
    let store = fixture.store();
    fs::rename(store.path(), fixture.base.join("outside")).unwrap();
    symlink(fixture.base.join("outside"), store.path()).unwrap();
    assert_eq!(
        store.load().unwrap_err(),
        NativeMcpConfigStoreError::UnsafePath
    );
    fs::remove_file(store.path()).unwrap();
    fixture.write("mcp.json", b"{}");
    let snapshot = store.load().unwrap();
    symlink(
        fixture.base.join("outside"),
        fixture.directory().join(".mcp.lock"),
    )
    .unwrap();
    assert_eq!(
        block_on(store.apply(&snapshot, &insert("new"))).unwrap_err(),
        NativeMcpConfigStoreError::UnsafePath
    );
    fs::remove_file(fixture.directory().join(".mcp.lock")).unwrap();
    fs::rename(fixture.directory(), fixture.base.join("moved")).unwrap();
    symlink(fixture.base.join("moved"), fixture.directory()).unwrap();
    assert_eq!(
        block_on(store.apply(&snapshot, &insert("new"))).unwrap_err(),
        NativeMcpConfigStoreError::UnsafePath
    );
    assert_eq!(fs::read(fixture.base.join("outside")).unwrap(), b"{}");
}

#[test]
fn replaced_ancestor_rejects_before_creation() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    let moved = fixture.base.with_extension("moved");
    fs::rename(&fixture.base, &moved).unwrap();
    fs::create_dir(&fixture.base).unwrap();
    let result = block_on(store.apply(&snapshot, &insert("new")));
    assert_eq!(result.unwrap_err(), NativeMcpConfigStoreError::Conflict);
    assert!(!fixture.base.join("missing").exists());
    assert!(!moved.join("missing").exists());
    fs::remove_dir(&moved).unwrap();
}

#[test]
fn busy_mcp_lock_is_nonblocking_and_settings_lock_is_independent() {
    let fixture = Fixture::new();
    fixture.seed(b"{}");
    let store = fixture.store();
    let settings_lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(fixture.directory().join(".config.lock"))
        .unwrap();
    rustix::fs::flock(
        &settings_lock,
        rustix::fs::FlockOperation::NonBlockingLockExclusive,
    )
    .unwrap();
    block_on(store.apply(&store.load().unwrap(), &insert("first"))).unwrap();
    let mcp_lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(fixture.directory().join(".mcp.lock"))
        .unwrap();
    rustix::fs::flock(
        &mcp_lock,
        rustix::fs::FlockOperation::NonBlockingLockExclusive,
    )
    .unwrap();
    assert_eq!(
        block_on(store.apply(&store.load().unwrap(), &insert("second"))).unwrap_err(),
        NativeMcpConfigStoreError::Busy
    );
    rustix::fs::flock(&mcp_lock, rustix::fs::FlockOperation::Unlock).unwrap();
    rustix::fs::flock(&settings_lock, rustix::fs::FlockOperation::Unlock).unwrap();
    assert_eq!(store.load().unwrap().config().servers().len(), 1);
}

#[test]
fn foreign_staging_file_is_preserved_without_replacing_data() {
    let fixture = Fixture::new();
    fixture.seed(b"{}");
    fixture.write(".mcp.tmp", b"foreign");
    let store = fixture.store();
    assert_eq!(
        block_on(store.apply(&store.load().unwrap(), &insert("new"))).unwrap_err(),
        NativeMcpConfigStoreError::Persistence
    );
    assert_eq!(
        fs::read(fixture.directory().join(".mcp.tmp")).unwrap(),
        b"foreign"
    );
    assert_eq!(fs::read(store.path()).unwrap(), b"{}");
}

#[test]
fn malformed_and_over_limit_files_are_preserved() {
    let fixture = Fixture::new();
    let store = fixture.store();
    for bytes in [
        b"not-json".to_vec(),
        vec![b' '; super::super::config::MAX_CONFIG_BYTES + 1],
        br#"{"mcp":{},"mcp":{}}"#.to_vec(),
    ] {
        fixture.seed(&bytes);
        assert!(matches!(
            store.load(),
            Err(NativeMcpConfigStoreError::InvalidConfig(_))
        ));
        assert_eq!(fs::read(store.path()).unwrap(), bytes);
        assert!(!fixture.directory().join(".mcp.lock").exists());
    }
    let mut exact = b"{}".to_vec();
    exact.resize(super::super::config::MAX_CONFIG_BYTES, b' ');
    fixture.seed(&exact);
    assert!(store.load().is_ok());
}

#[test]
fn invalid_and_aggregate_overflow_proposals_have_no_publication_effects() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    for name in [
        String::new(),
        "bad/name".to_owned(),
        "x".repeat(MAX_SERVER_NAME_BYTES + 1),
    ] {
        assert!(matches!(
            block_on(store.apply(&snapshot, &McpConfigMutation::Remove(name.into()))),
            Err(NativeMcpConfigStoreError::InvalidConfig(_))
        ));
    }
    assert!(!fixture.base.join("missing").exists());
    let arg = "x".repeat(16 * 1024);
    let first = McpServerConfig::stdio("first", "node", &vec![arg.as_str(); 20]).unwrap();
    let second = McpServerConfig::stdio("second", "node", &vec![arg.as_str(); 20]).unwrap();
    let mut config = McpConfig::new();
    config.insert(first).unwrap();
    fixture.seed(&config.encode().unwrap());
    let before = fs::read(store.path()).unwrap();
    let result = block_on(store.apply(&store.load().unwrap(), &McpConfigMutation::Insert(second)));
    assert_eq!(
        result.unwrap_err(),
        NativeMcpConfigStoreError::InvalidConfig(McpConfigError::Limit)
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert!(!fixture.directory().join(".mcp.lock").exists());
}

#[test]
fn postrename_sync_failure_retains_intended_receipt_and_requires_reconciliation() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    let receipt = store
        .publish(&snapshot, &insert("new"), |_| Err(rustix::io::Errno::IO))
        .unwrap();
    assert!(receipt.changed());
    assert_eq!(receipt.durability(), McpConfigCommitDurability::Ambiguous);
    assert_eq!(store.load().unwrap().config(), receipt.intended());
    assert_eq!(
        block_on(store.apply(&snapshot, &insert("new"))).unwrap_err(),
        NativeMcpConfigStoreError::Conflict
    );
    assert!(!fixture.directory().join(".mcp.tmp").exists());
}

#[test]
fn moved_root_after_rename_returns_ambiguous_without_rollback() {
    let fixture = Fixture::new();
    fixture.seed(b"{}");
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    let moved = fixture.base.join("moved");
    let receipt = store
        .publish(&snapshot, &insert("new"), |root| {
            fs::rename(fixture.directory(), &moved).unwrap();
            fs::create_dir(fixture.directory()).unwrap();
            rustix::fs::fsync(root)
        })
        .unwrap();
    assert_eq!(receipt.durability(), McpConfigCommitDurability::Ambiguous);
    assert_eq!(
        fs::read(moved.join("mcp.json")).unwrap(),
        receipt.intended().encode().unwrap()
    );
    assert!(!store.path().exists());
}

#[test]
fn replaced_published_data_is_ambiguous_even_when_candidate_bytes_match() {
    let fixture = Fixture::new();
    fixture.seed(b"{}");
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    let receipt = store
        .publish(&snapshot, &insert("new"), |root| {
            let bytes = fs::read(store.path()).unwrap();
            fixture.write("replacement", &bytes);
            fs::rename(fixture.directory().join("replacement"), store.path()).unwrap();
            rustix::fs::fsync(root)
        })
        .unwrap();
    assert_eq!(receipt.durability(), McpConfigCommitDurability::Ambiguous);
    assert_eq!(store.load().unwrap().config(), receipt.intended());
}

#[test]
fn namespace_selection_never_touches_sibling_settings_or_fx_fixture() {
    let fixture = Fixture::new();
    fixture.seed(b"{}");
    fixture.write("config.json", b"leave settings untouched");
    fixture.write(".config.tmp", b"leave settings staging untouched");
    let fx = fixture.base.join("fx-profile");
    fs::create_dir(&fx).unwrap();
    fs::write(fx.join("mcp.json"), b"leave fx untouched").unwrap();
    let store = fixture.store();
    block_on(store.apply(&store.load().unwrap(), &insert("new"))).unwrap();
    assert_eq!(
        fs::read(fixture.directory().join("config.json")).unwrap(),
        b"leave settings untouched"
    );
    assert_eq!(
        fs::read(fixture.directory().join(".config.tmp")).unwrap(),
        b"leave settings staging untouched"
    );
    assert_eq!(
        fs::read(fx.join("mcp.json")).unwrap(),
        b"leave fx untouched"
    );
}

#[test]
fn invalid_paths_and_debug_are_bounded_and_redacted() {
    for path in [
        PathBuf::from("relative"),
        PathBuf::from("/"),
        PathBuf::from("/a/../b"),
        PathBuf::from(format!("/{}", "x".repeat(4096))),
        PathBuf::from(format!("/{}", "a/".repeat(65))),
    ] {
        assert_eq!(
            NativeMcpConfigStore::new(path).unwrap_err(),
            NativeMcpConfigStoreError::UnsafePath
        );
    }
    let fixture = Fixture::new();
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    let mutation = McpConfigMutation::Remove("secret_alias".into());
    let receipt = block_on(store.apply(&snapshot, &mutation)).unwrap();
    let debug = format!("{store:?} {snapshot:?} {mutation:?} {receipt:?}");
    assert!(!debug.contains("secret_alias"));
    assert!(!debug.contains(&fixture.base.display().to_string()));
}

#[test]
fn concurrent_mcp_instances_have_one_cas_winner() {
    let fixture = Fixture::new();
    fixture.seed(b"{}");
    let stores = [fixture.store(), fixture.store()];
    let snapshots = stores.each_ref().map(|store| store.load().unwrap());
    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            barrier.wait();
            block_on(stores[0].apply(&snapshots[0], &insert("first")))
        });
        let second = scope.spawn(|| {
            barrier.wait();
            block_on(stores[1].apply(&snapshots[1], &insert("second")))
        });
        [first.join().unwrap(), second.join().unwrap()]
    });
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    for result in results.into_iter().filter(Result::is_err) {
        assert!(matches!(
            result,
            Err(NativeMcpConfigStoreError::Busy | NativeMcpConfigStoreError::Conflict)
        ));
    }
    assert_eq!(fixture.store().load().unwrap().config().servers().len(), 1);
}
