use super::*;
use crate::mcp::controller::{NativeMcpControllerError, NativeMcpControllerPublication};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    directory: PathBuf,
    workers: NativeOwnedWorkerScope,
    clock: Arc<Clock>,
    runtime: Arc<NativeMcpRuntime>,
    controller: Arc<NativeMcpController>,
}
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "mg-mcp-control-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let directory = fs::canonicalize(directory).unwrap();
        let workers = NativeOwnedWorkerScope::new();
        let (runtime, clock) = runtime();
        let controller = Arc::new(
            NativeMcpController::new(NativeMcpControllerOptions {
                runtime: runtime.clone(),
                management: Arc::new(NativeMcpManagementService::new(Arc::new(
                    NativeMcpConfigStore::new(directory.join("profile")).unwrap(),
                ))),
                workers: workers.clone(),
                reserved_tool_names: Box::new([]),
                startup: NativeMcpControllerStartupOptions {
                    captured_environment: vec![],
                    stdio: None,
                    clock: clock.clone(),
                    catalog_epoch: clock.instant,
                    owner_cancellation: CancellationToken::new(),
                    #[cfg(feature = "mcp-http")]
                    network: None,
                    #[cfg(feature = "mcp-http")]
                    authentication: vec![],
                    peer_lifetime: crate::mcp::lifetime::McpPeerLifetime::Until(
                        clock.instant + Duration::from_secs(600),
                    ),
                    max_retained_bytes: 1024 * 1024,
                },
                max_retained_generations: 4,
                #[cfg(feature = "mcp-http")]
                stored_authentication: None,
            })
            .unwrap(),
        );
        Self {
            directory,
            workers,
            clock,
            runtime,
            controller,
        }
    }
    fn seed_invalid(&self) {
        let profile = self.directory.join("profile");
        fs::create_dir(&profile).unwrap();
        fs::set_permissions(&profile, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(profile.join("mcp.json"), "invalid selected profile").unwrap();
        fs::set_permissions(profile.join("mcp.json"), fs::Permissions::from_mode(0o600)).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.controller.close();
        self.workers.close();
        self.workers.completion().wait_on_worker().unwrap();
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

#[test]
fn reload_unpolled_precancelled_and_retired_never_observe_selected_clock_or_profile() {
    let fixture = Fixture::new();
    let conversation = conversation();
    let token = CancellationToken::new();
    let future = reload::run(
        conversation.clone(),
        fixture.controller.clone(),
        token.clone(),
    );
    assert_eq!(fixture.clock.reads.load(Ordering::Relaxed), 0);
    drop(future);
    assert!(token.is_cancelled());
    assert!(matches!(
        futures_executor::block_on(reload::run(
            conversation.clone(),
            fixture.controller.clone(),
            token
        )),
        Err(Error::Mcp(NativeMcpManagementError::Cancelled))
    ));
    conversation.begin_quiescence().unwrap().retire().unwrap();
    assert!(matches!(
        futures_executor::block_on(reload::run(
            conversation,
            fixture.controller.clone(),
            CancellationToken::new()
        )),
        Err(Error::Runtime(_))
    ));
    assert_eq!(fixture.clock.reads.load(Ordering::Relaxed), 0);
    assert!(!fixture.directory.join("profile").exists());
}

#[test]
fn reload_keeps_exact_publication_receipt_and_failed_reload_preserves_it() {
    let fixture = Fixture::new();
    let conversation = conversation();
    let token = CancellationToken::new();
    let Receipt::McpReload(receipt) = futures_executor::block_on(reload::run(
        conversation.clone(),
        fixture.controller.clone(),
        token.clone(),
    ))
    .unwrap() else {
        panic!("reload receipt")
    };
    assert_eq!(
        receipt.publication(),
        NativeMcpControllerPublication::Published
    );
    assert!(!receipt.closed_after_publication());
    assert!(
        token.is_cancelled(),
        "completion drops only the control cancellation guard"
    );
    assert_eq!(
        receipt.publication(),
        NativeMcpControllerPublication::Published
    );
    let checkpoint = fixture.runtime.publication_checkpoint().unwrap();
    let candidate = fixture.runtime.prepare_candidate(vec![], &[]).unwrap();
    fixture.seed_invalid();
    let Err(Error::McpReload(failure)) = futures_executor::block_on(reload::run(
        conversation.clone(),
        fixture.controller.clone(),
        CancellationToken::new(),
    )) else {
        panic!("typed reload failure")
    };
    assert!(matches!(failure.kind(), NativeMcpControllerError::Store(_)));
    assert!(
        fixture.runtime.publish_if(candidate, &checkpoint).is_ok(),
        "failed reload never changes prior exact publication"
    );
    let mut fence = conversation.begin_quiescence().unwrap();
    assert!(fence.try_retire().is_ok());
}
