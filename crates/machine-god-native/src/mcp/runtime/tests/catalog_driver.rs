use super::*;
use futures_executor::block_on;
use futures_util::FutureExt;
use std::sync::atomic::AtomicU64;

struct CatalogClock {
    epoch: Instant,
    milliseconds: AtomicU64,
}
impl NativeMcpRuntimeClock for CatalogClock {
    fn now(&self) -> Instant {
        self.epoch + Duration::from_millis(self.milliseconds.load(Ordering::Acquire))
    }
    fn sleep_until(&self, _: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}
struct Driver {
    runtime: NativeMcpRuntime,
    clock: Arc<CatalogClock>,
    writes: Arc<Mutex<Vec<u8>>>,
}
impl Driver {
    fn new(
        ttl: u64,
        response: impl Fn(i64) -> BoxFuture<'static, Result<Box<[u8]>>> + Send + Sync + 'static,
    ) -> Self {
        let clock = Arc::new(CatalogClock {
            epoch: Instant::now(),
            milliseconds: AtomicU64::new(0),
        });
        let mut runtime = standalone();
        runtime.clock = clock.clone();
        let writes = Arc::default();
        let mut server = addition::server("selected", &["old"]);
        server.catalog_epoch = clock.epoch;
        let mut builder = McpCatalogBuilder::new(
            McpCatalogKind::Tools,
            ProtocolVersion::Modern,
            McpCatalogLimits::default(),
        )
        .unwrap();
        builder
            .append_response(&reply(1, &["old"], ttl), &RpcId::Integer(1), None, 0)
            .unwrap();
        server.catalogs = vec![
            McpDescriptorCatalog::admit(builder.finish().unwrap(), McpDescriptorLimits::default())
                .unwrap(),
        ];
        server.peer = NativeMcpOwnedPeer::Script(
            script::ScriptPeer::new(Arc::clone(&writes)).with_catalog_response(response),
        );
        runtime
            .publish(runtime.prepare_candidate(vec![server], &[]).unwrap())
            .unwrap();
        Self {
            runtime,
            clock,
            writes,
        }
    }
    fn advance(&self, milliseconds: u64) {
        self.clock
            .milliseconds
            .store(milliseconds, Ordering::Release);
    }
    fn refresh(&self) -> Result<()> {
        block_on(self.runtime.refresh_for_human(
            "selected",
            &CancellationToken::new(),
            &CancellationToken::new(),
        ))
    }
    fn active(&self) -> Arc<super::super::candidate::Publication> {
        self.runtime.state.lock().unwrap().active.clone().unwrap()
    }
    fn requests(&self) -> Vec<Value> {
        self.writes
            .lock()
            .unwrap()
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| machine_god_core::json::from_slice(line).unwrap())
            .collect()
    }
}
fn reply(id: i64, names: &[&str], ttl: u64) -> Box<[u8]> {
    serde_json::to_vec(&json!({"jsonrpc":"2.0", "id":id, "result":{
        "resultType":"complete", "ttlMs":ttl,
        "tools":names.iter().map(|name| json!({"name":name,"inputSchema":{"type":"object"}})).collect::<Vec<_>>()
    }})).unwrap().into()
}

#[test]
fn ttl_hit_is_inert_then_expiry_rebinds_on_the_same_peer_with_fresh_ids() {
    let driver = Driver::new(100, |id| {
        Box::pin(async move { Ok(reply(id, &["new"], 100)) })
    });
    let original = driver.active();
    driver.refresh().unwrap();
    driver.advance(99);
    driver.refresh().unwrap();
    assert!(driver.requests().is_empty());
    driver.advance(100);
    driver.refresh().unwrap();
    let current = driver.active();
    assert!(!Arc::ptr_eq(&original, &current));
    assert!(Arc::ptr_eq(&original.servers[0], &current.servers[0]));
    assert!(
        original
            .tools
            .values()
            .all(|tool| tool.binding.live().is_err())
    );
    assert!(
        current
            .tools
            .values()
            .all(|tool| tool.descriptor.name() == "new" && tool.binding.live().is_ok())
    );
    driver.advance(199);
    driver.refresh().unwrap();
    assert_eq!(driver.requests().len(), 1);
    driver.advance(200);
    driver.refresh().unwrap();
    let requests = driver.requests();
    assert_eq!(requests.len(), 2);
    assert_ne!(requests[0]["id"], requests[1]["id"]);
    assert!(
        requests
            .iter()
            .all(|request| request["method"] == "tools/list"
                && request["params"]["_meta"].is_object())
    );
}

#[test]
fn unchanged_ttl_refresh_keeps_original_payload_and_registration_allocations() {
    let driver = Driver::new(0, |id| {
        Box::pin(async move { Ok(reply(id, &["old"], 100)) })
    });
    let original = driver.active();
    let cached = original.servers[0]
        .catalogs
        .lock()
        .unwrap()
        .cached(McpCatalogKind::Tools)
        .unwrap();
    let registration = original.snapshot.tools()[0].executable().unwrap();
    driver.refresh().unwrap();
    assert!(Arc::ptr_eq(&original, &driver.active()));
    assert!(Arc::ptr_eq(
        &registration,
        &driver.active().snapshot.tools()[0].executable().unwrap()
    ));
    let current = original.servers[0]
        .catalogs
        .lock()
        .unwrap()
        .cached(McpCatalogKind::Tools)
        .unwrap();
    assert!(cached.same_allocation(&current));
    driver.advance(99);
    driver.refresh().unwrap();
    assert_eq!(driver.requests().len(), 1);
    assert!(
        driver
            .runtime
            .state
            .lock()
            .unwrap()
            .retired_catalogs
            .is_empty()
    );
}

#[test]
fn failed_refresh_serves_existing_snapshot_and_obeys_bounded_retry_delay() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = attempts.clone();
    let driver = Driver::new(0, move |id| {
        let attempt = counter.fetch_add(1, Ordering::AcqRel);
        Box::pin(async move {
            if attempt == 0 {
                Err(NativeMcpRuntimeError::Unavailable)
            } else {
                Ok(reply(id, &["new"], 100))
            }
        })
    });
    let original = driver.active();
    driver.refresh().unwrap();
    assert!(Arc::ptr_eq(&original, &driver.active()));
    driver.advance(99);
    driver.refresh().unwrap();
    assert_eq!(attempts.load(Ordering::Acquire), 1);
    driver.advance(100);
    driver.refresh().unwrap();
    assert_eq!(attempts.load(Ordering::Acquire), 2);
    assert!(!Arc::ptr_eq(&original, &driver.active()));
}

#[test]
fn pending_human_cancellation_and_abandonment_release_refresh_ticket_without_replay() {
    for abandon in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let driver = Driver::new(0, move |id| {
            if counter.fetch_add(1, Ordering::AcqRel) == 0 {
                Box::pin(std::future::pending())
            } else {
                Box::pin(async move { Ok(reply(id, &["new"], 100)) })
            }
        });
        let original = driver.active();
        let command = CancellationToken::new();
        let cancellation = CancellationToken::new();
        let mut pending = Box::pin(driver.runtime.refresh_for_human(
            "selected",
            &command,
            &cancellation,
        ));
        assert!(pending.as_mut().now_or_never().is_none());
        assert_eq!(calls.load(Ordering::Acquire), 1);
        if abandon {
            drop(pending);
        } else {
            cancellation.cancel();
            assert_eq!(block_on(pending), Err(NativeMcpRuntimeError::Cancelled));
        }
        assert!(Arc::ptr_eq(&original, &driver.active()));
        assert!(
            original
                .tools
                .values()
                .all(|tool| tool.binding.live().is_ok())
        );
        driver.refresh().unwrap();
        assert_eq!(calls.load(Ordering::Acquire), 2);
        assert!(!Arc::ptr_eq(&original, &driver.active()));
        let requests = driver.requests();
        assert_ne!(requests[0]["id"], requests[1]["id"]);
    }
}

#[test]
fn cancellation_inside_ready_catalog_reply_cannot_publish_for_original_human() {
    let command = CancellationToken::new();
    let stop = command.clone();
    let driver = Driver::new(0, move |id| {
        stop.cancel();
        Box::pin(async move { Ok(reply(id, &["new"], 100)) })
    });
    let original = driver.active();
    assert!(
        block_on(
            driver
                .runtime
                .refresh_for_human("selected", &command, &CancellationToken::new())
        )
        .is_err()
    );
    assert!(Arc::ptr_eq(&original, &driver.active()));
    assert!(
        original
            .tools
            .values()
            .all(|tool| tool.binding.live().is_ok())
    );
}

#[test]
fn original_turn_retirement_during_ready_reply_cannot_publish_or_pin_replacements() {
    let selected: Arc<Mutex<Option<Arc<McpSubmissionRegistry>>>> = Arc::default();
    let retire = selected.clone();
    let driver = Driver::new(0, move |id| {
        retire.lock().unwrap().as_ref().unwrap().retire();
        Box::pin(async move { Ok(reply(id, &["new"], 100)) })
    });
    let (_engine, _conversation, _turn, context) =
        addition::conversation(&driver.runtime, "catalog-turn");
    let native = driver.runtime.contexts.snapshot_for_tool(&context).unwrap();
    *selected.lock().unwrap() = Some(native.registry().unwrap());
    let original = driver.active();
    assert!(
        block_on(
            driver
                .runtime
                .refresh_for_turn(&native, &CancellationToken::new())
        )
        .is_err()
    );
    assert!(Arc::ptr_eq(&original, &driver.active()));
    assert!(driver.runtime.state.lock().unwrap().turns.is_empty());
}

#[test]
fn malformed_correlated_catalog_does_not_replace_live_descriptors() {
    let driver = Driver::new(0, |id| {
        Box::pin(async move { Ok(reply(id + 1, &["foreign"], 100)) })
    });
    let original = driver.active();
    driver.refresh().unwrap();
    assert!(Arc::ptr_eq(&original, &driver.active()));
    assert!(
        original
            .tools
            .values()
            .all(|tool| tool.binding.live().is_ok())
    );
    driver.advance(99);
    driver.refresh().unwrap();
    assert_eq!(driver.requests().len(), 1);
}
