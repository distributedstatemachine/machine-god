//! Human-command cache driving with concrete, effect-free scripted peers.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64};

struct CacheClock {
    epoch: Instant,
    millis: AtomicU64,
}
impl CacheClock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            epoch: Instant::now(),
            millis: AtomicU64::new(0),
        })
    }
    fn advance(&self, millis: u64) {
        assert!(millis >= self.millis.load(Ordering::Acquire));
        self.millis.store(millis, Ordering::Release);
    }
}
impl NativeMcpRuntimeClock for CacheClock {
    fn now(&self) -> Instant {
        self.epoch + Duration::from_millis(self.millis.load(Ordering::Acquire))
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::poll_fn(move |_| {
            if self.now() >= deadline {
                std::task::Poll::Ready(())
            } else {
                std::task::Poll::Pending
            }
        }))
    }
}

struct Producer {
    ttl: u64,
    description_bytes: usize,
    text: &'static str,
    fail_resources: AtomicBool,
    cancel_list: Mutex<Option<CancellationToken>>,
}
impl Producer {
    fn new(ttl: u64, description_bytes: usize, text: &'static str) -> Arc<Self> {
        Arc::new(Self {
            ttl,
            description_bytes,
            text,
            fail_resources: AtomicBool::new(false),
            cancel_list: Mutex::new(None),
        })
    }
    fn reply(&self, id: i64, request: &Value) -> Box<[u8]> {
        let method = request["method"].as_str().unwrap();
        if method == "resources/list" {
            let cancellation = self.cancel_list.lock().unwrap().take();
            if let Some(cancellation) = cancellation {
                cancellation.cancel();
            }
            if self.fail_resources.load(Ordering::Acquire) {
                return serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":"fixture catalog unavailable"}})).unwrap().into();
            }
        }
        let mut result = match method {
            "resources/list" => {
                json!({"resources":[{"uri":"test://fixed","name":"fixed","description":"x".repeat(self.description_bytes)}]})
            }
            "resources/templates/list" => {
                json!({"resourceTemplates":[{"uriTemplate":"test:///{id}","name":"dynamic"}]})
            }
            "prompts/list" => {
                json!({"prompts":[{"name":"review","description":"x".repeat(self.description_bytes),"arguments":[{"name":"topic","required":true}]}]})
            }
            "resources/read" => {
                json!({"contents":[{"uri":request["params"]["uri"],"text":self.text}]})
            }
            "prompts/get" => {
                json!({"messages":[{"role":"assistant","content":{"type":"text","text":self.text}}]})
            }
            "completion/complete" => {
                json!({"completion":{"values":["one"],"total":1,"hasMore":false}})
            }
            _ => panic!("unexpected cache fixture method"),
        };
        result["resultType"] = json!("complete");
        if matches!(
            method,
            "resources/list" | "resources/templates/list" | "prompts/list"
        ) {
            result["ttlMs"] = json!(self.ttl);
        }
        serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"result":result}))
            .unwrap()
            .into()
    }
}

fn cached_runtime(clock: Arc<CacheClock>, maximum: usize) -> Arc<NativeMcpRuntime> {
    let mut runtime = standalone_with_limits(NativeMcpRuntimeLimits {
        max_retained_bytes: maximum,
        ..NativeMcpRuntimeLimits::default()
    });
    runtime.clock = clock;
    Arc::new(runtime)
}

fn install_cache(runtime: &NativeMcpRuntime, producer: Arc<Producer>) -> Arc<Mutex<Vec<u8>>> {
    let writes = Arc::<Mutex<Vec<u8>>>::default();
    let source = writes.clone();
    let mut peer = script::ScriptPeer::new(writes.clone()).with_response(move |id| {
        let request = {
            let wire = source.lock().unwrap();
            let last = wire
                .split(|byte| *byte == b'\n')
                .rfind(|line| !line.is_empty())
                .unwrap();
            machine_god_core::json::from_slice(last).unwrap()
        };
        producer.reply(id, &request)
    });
    peer.tools = false;
    let candidate = runtime
        .prepare_candidate(
            vec![NativeMcpServerCandidate {
                server: Arc::from("fixture"),
                configuration: Arc::from(&b"configuration"[..]),
                authentication: Arc::from(&b"authentication"[..]),
                catalogs: vec![],
                refresh: None,
                catalog_epoch: runtime.clock.now(),
                peer: NativeMcpOwnedPeer::Script(peer),
                operation_timeout: Duration::from_secs(120),
                authority_cancellations: Arc::from([]),
            }],
            &[],
        )
        .unwrap();
    runtime.publish(candidate).unwrap();
    writes
}

fn methods(writes: &Mutex<Vec<u8>>) -> Vec<String> {
    sent(writes)
        .iter()
        .map(|value| value["method"].as_str().unwrap().to_owned())
        .collect()
}
fn count(writes: &Mutex<Vec<u8>>, method: &str) -> usize {
    methods(writes)
        .iter()
        .filter(|actual| actual.as_str() == method)
        .count()
}
fn execute(owner: &NativeMcpHumanCommand, command: &str) {
    futures_executor::block_on(owner.feature(&request(command), CancellationToken::new())).unwrap();
}
const READ: &str = "resource read fixture test://fixed";
const GET: &str = r#"prompt get fixture review {"topic":"rust"}"#;

#[test]
fn positive_ttl_reuses_all_lazy_prerequisites_but_direct_lists_stay_explicit() {
    let clock = CacheClock::new();
    let runtime = cached_runtime(
        clock.clone(),
        NativeMcpRuntimeLimits::default().max_retained_bytes,
    );
    let writes = install_cache(&runtime, Producer::new(100, 0, "original"));
    let owner = runtime.human_command();
    for _ in 0..2 {
        execute(&owner, READ);
        execute(&owner, GET);
        execute(&owner, "prompt complete fixture review topic ru");
        execute(&owner, "resource complete fixture test:///{id} id ru");
    }
    assert_eq!(count(&writes, "resources/list"), 1);
    assert_eq!(count(&writes, "prompts/list"), 1);
    assert_eq!(count(&writes, "resources/templates/list"), 1);
    assert_eq!(count(&writes, "resources/read"), 2);
    assert_eq!(count(&writes, "prompts/get"), 2);
    assert_eq!(count(&writes, "completion/complete"), 4);
    clock.advance(99);
    execute(&owner, READ);
    assert_eq!(count(&writes, "resources/list"), 1);
    execute(&owner, "resource list fixture");
    execute(&owner, "resource list fixture");
    assert_eq!(count(&writes, "resources/list"), 3);
    clock.advance(100);
    execute(&owner, READ);
    assert_eq!(count(&writes, "resources/list"), 4);
    for (index, request) in sent(&writes).iter().enumerate() {
        assert_eq!(request["id"].as_u64(), Some(index as u64 + 1));
    }
    assert_eq!(runtime.feature_operations.load(Ordering::Acquire), 0);
}

#[test]
fn expired_catalog_failure_keeps_same_partition_data_and_bounded_retry_backoff() {
    let clock = CacheClock::new();
    let runtime = cached_runtime(
        clock.clone(),
        NativeMcpRuntimeLimits::default().max_retained_bytes,
    );
    let producer = Producer::new(10, 0, "original");
    let writes = install_cache(&runtime, producer.clone());
    let owner = runtime.human_command();
    execute(&owner, READ);
    producer.fail_resources.store(true, Ordering::Release);
    for (time, lists) in [(10, 2), (109, 2), (110, 3), (309, 3)] {
        clock.advance(time);
        let response =
            futures_executor::block_on(owner.feature(&request(READ), CancellationToken::new()))
                .unwrap();
        let McpFeatureReply::Response(response) = response.reply() else {
            panic!("resource response")
        };
        assert!(response.raw_json().get().contains("original"));
        assert_eq!(count(&writes, "resources/list"), lists);
    }
    producer.fail_resources.store(false, Ordering::Release);
    clock.advance(310);
    execute(&owner, READ);
    assert_eq!(count(&writes, "resources/list"), 4);
    assert_eq!(count(&writes, "resources/read"), 6);
    assert_eq!(runtime.feature_operations.load(Ordering::Acquire), 0);
    // ScriptPeer stamps lists at its selected epoch; this checks retry recovery,
    // not an extension of refreshed fetch timestamps in the synthetic peer.
}

#[test]
fn cancellation_during_refresh_never_falls_back_to_the_cached_descriptor() {
    for failed_reply in [false, true] {
        let clock = CacheClock::new();
        let runtime = cached_runtime(
            clock.clone(),
            NativeMcpRuntimeLimits::default().max_retained_bytes,
        );
        let producer = Producer::new(10, 0, "original");
        let writes = install_cache(&runtime, producer.clone());
        let owner = runtime.human_command();
        execute(&owner, READ);
        let precancelled = CancellationToken::new();
        precancelled.cancel();
        assert!(futures_executor::block_on(owner.feature(&request(READ), precancelled)).is_err());
        assert_eq!(
            sent(&writes).len(),
            2,
            "a fresh cache cannot extend cancelled authority"
        );
        clock.advance(10);
        let cancellation = CancellationToken::new();
        *producer.cancel_list.lock().unwrap() = Some(cancellation.clone());
        producer
            .fail_resources
            .store(failed_reply, Ordering::Release);
        assert!(futures_executor::block_on(owner.feature(&request(READ), cancellation)).is_err());
        assert_eq!(
            methods(&writes),
            ["resources/list", "resources/read", "resources/list"]
        );
        assert_eq!(runtime.feature_operations.load(Ordering::Acquire), 0);
    }
}

#[test]
fn shared_cache_pressure_keeps_fresh_results_usable_without_unbounded_retention() {
    let clock = CacheClock::new();
    let runtime = cached_runtime(clock.clone(), 8192);
    let writes = install_cache(&runtime, Producer::new(1000, 1000, "pressure"));
    let owner = runtime.human_command();
    execute(&owner, READ);
    execute(&owner, GET);
    let publication = runtime.state.lock().unwrap().active.clone().unwrap();
    {
        let state = publication.servers[0].catalogs.lock().unwrap();
        let resource = state.cached(McpCatalogKind::Resources).unwrap();
        assert!(resource.retained_byte_charge() <= 8192);
        assert!(state.cached(McpCatalogKind::Prompts).is_none());
    }
    let before = sent(&writes).len();
    assert!(
        futures_executor::block_on(owner.feature(&request(GET), CancellationToken::new())).is_err()
    );
    assert_eq!(
        sent(&writes).len(),
        before,
        "missing pressure snapshot honors backoff without a replay"
    );
    execute(&owner, READ);
    assert_eq!(count(&writes, "resources/list"), 1);
    clock.advance(100);
    execute(&owner, GET);
    assert_eq!(count(&writes, "prompts/list"), 2);
    assert_eq!(count(&writes, "prompts/get"), 2);
    assert!(
        publication.servers[0]
            .catalogs
            .lock()
            .unwrap()
            .cached(McpCatalogKind::Prompts)
            .is_none()
    );
    assert_eq!(runtime.feature_operations.load(Ordering::Acquire), 0);
}

#[test]
fn cached_catalogs_never_cross_a_replaced_runtime_partition() {
    let clock = CacheClock::new();
    let runtime = cached_runtime(clock, NativeMcpRuntimeLimits::default().max_retained_bytes);
    let old = install_cache(&runtime, Producer::new(10_000, 0, "old partition"));
    let owner = runtime.human_command();
    execute(&owner, READ);
    let replacement = Producer::new(10_000, 0, "new partition");
    replacement.fail_resources.store(true, Ordering::Release);
    let new = install_cache(&runtime, replacement);
    assert!(
        futures_executor::block_on(owner.feature(&request(READ), CancellationToken::new()))
            .is_err()
    );
    assert_eq!(methods(&old), ["resources/list", "resources/read"]);
    assert_eq!(methods(&new), ["resources/list"]);
    assert_eq!(runtime.feature_operations.load(Ordering::Acquire), 0);
}
