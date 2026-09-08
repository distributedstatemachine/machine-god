use std::collections::VecDeque;
use std::future::{Future, pending};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Instant;

use futures_executor::block_on;
use machine_god_core::{BoxFuture, CancellationToken};
use machine_god_native::{
    AiGatewayModelCatalogAccessMode, AiGatewayModelCatalogProvider,
    AiGatewayModelCatalogRequestAccess, AiGatewayModelCatalogTransport,
    AiGatewayModelCatalogTransportError, AiGatewayModelCatalogTransportResponse,
    NATIVE_MODEL_CATALOG_MAX_WAITERS, NativeModelCatalogCache, NativeModelCatalogCacheError,
    NativeModelCatalogCacheState,
};

const RICH: &[u8] = br#"{"data":[{"id":"private/model","reasoning_options":[{"type":"effort","values":["high"]}],"fast_options":[{"type":"toggle"}]}]}"#;
const EMPTY: &[u8] = br#"{"data":[]}"#;

enum Reply {
    Response(u16, &'static [u8]),
    Pending(Arc<AtomicUsize>),
    Gated(Arc<AtomicBool>),
}

struct Transport {
    replies: Mutex<VecDeque<Reply>>,
    calls: AtomicUsize,
}

struct PendingRequest(Arc<AtomicUsize>);
impl Future for PendingRequest {
    type Output =
        Result<AiGatewayModelCatalogTransportResponse, AiGatewayModelCatalogTransportError>;
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}
impl Drop for PendingRequest {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl AiGatewayModelCatalogTransport for Transport {
    fn wait_until(&self, _: Instant) -> BoxFuture<'_, ()> {
        Box::pin(pending())
    }
    fn get(
        &self,
        _: AiGatewayModelCatalogRequestAccess,
        _: Instant,
        _: CancellationToken,
    ) -> BoxFuture<
        '_,
        Result<AiGatewayModelCatalogTransportResponse, AiGatewayModelCatalogTransportError>,
    > {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted reply")
        {
            Reply::Response(status, body) => Box::pin(async move {
                Ok(AiGatewayModelCatalogTransportResponse::new(
                    status,
                    body.to_vec(),
                ))
            }),
            Reply::Pending(drops) => Box::pin(PendingRequest(drops)),
            Reply::Gated(ready) => Box::pin(std::future::poll_fn(move |_| {
                if ready.load(Ordering::SeqCst) {
                    Poll::Ready(Ok(AiGatewayModelCatalogTransportResponse::new(
                        200,
                        RICH.to_vec(),
                    )))
                } else {
                    Poll::Pending
                }
            })),
        }
    }
}

fn cache(replies: impl IntoIterator<Item = Reply>) -> (NativeModelCatalogCache, Arc<Transport>) {
    let transport = Arc::new(Transport {
        replies: Mutex::new(replies.into_iter().collect()),
        calls: AtomicUsize::new(0),
    });
    let provider = Arc::new(AiGatewayModelCatalogProvider::new(
        AiGatewayModelCatalogAccessMode::PublicOnly,
        transport.clone(),
    ));
    (NativeModelCatalogCache::new(provider), transport)
}

fn poll<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

#[test]
fn construction_and_unpolled_load_are_inert_and_ready_success_has_no_ttl() {
    let (cache, transport) = cache([Reply::Response(200, RICH)]);
    drop(cache.load(20, CancellationToken::new()));
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    assert_eq!(cache.snapshot().state, NativeModelCatalogCacheState::Idle);
    let first = block_on(cache.load(20, CancellationToken::new())).unwrap();
    let again = block_on(cache.load(u64::MAX, CancellationToken::new())).unwrap();
    assert!(Arc::ptr_eq(
        first.catalog.as_ref().unwrap(),
        again.catalog.as_ref().unwrap()
    ));
    assert_eq!(again.last_attempt_ms, Some(20));
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    let details = again
        .catalog
        .as_ref()
        .unwrap()
        .details("private/model")
        .unwrap();
    assert!(details.capabilities().supports_fast());
    assert_eq!(
        details.capabilities().reasoning_efforts()[0].as_named(),
        Some("high")
    );
    assert!(!format!("{cache:?} {again:?}").contains("private/model"));
}

#[test]
fn failed_without_catalog_retries_even_nonretryable_at_exact_start_time_boundary() {
    let (cache, transport) = cache([Reply::Response(400, EMPTY), Reply::Response(200, RICH)]);
    let failed = block_on(cache.load(500, CancellationToken::new())).unwrap();
    assert_eq!(failed.state, NativeModelCatalogCacheState::Failed);
    assert!(!failed.last_failure.unwrap().retryable);
    for now in [0, 499, 500, 1499] {
        block_on(cache.load(now, CancellationToken::new())).unwrap();
    }
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        block_on(cache.load(1500, CancellationToken::new()))
            .unwrap()
            .state,
        NativeModelCatalogCacheState::Ready
    );
}

#[test]
fn retained_retryable_failure_retries_but_retained_nonretryable_does_not() {
    let (cache, transport) = cache([
        Reply::Response(200, RICH),
        Reply::Response(503, EMPTY),
        Reply::Response(400, EMPTY),
        Reply::Response(200, EMPTY),
    ]);
    let original = block_on(cache.load(0, CancellationToken::new()))
        .unwrap()
        .catalog
        .unwrap();
    let failure = block_on(cache.refresh(1, CancellationToken::new())).unwrap();
    assert_eq!(failure.state, NativeModelCatalogCacheState::Ready);
    assert!(failure.last_failure.unwrap().retryable);
    assert!(Arc::ptr_eq(&original, failure.catalog.as_ref().unwrap()));
    block_on(cache.load(1000, CancellationToken::new())).unwrap();
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
    let failure = block_on(cache.load(1001, CancellationToken::new())).unwrap();
    assert!(!failure.last_failure.unwrap().retryable);
    block_on(cache.load(u64::MAX, CancellationToken::new())).unwrap();
    assert_eq!(transport.calls.load(Ordering::SeqCst), 3);
    let empty = block_on(cache.refresh(1002, CancellationToken::new())).unwrap();
    assert!(Arc::ptr_eq(&original, empty.catalog.as_ref().unwrap()));
    assert_eq!(empty.last_failure, None);
    assert_eq!(empty.state, NativeModelCatalogCacheState::Ready);
}

#[test]
fn empty_initial_success_is_cached_and_timestamp_overflow_cannot_create_retry() {
    let (cache, transport) = cache([Reply::Response(200, EMPTY), Reply::Response(503, EMPTY)]);
    assert!(
        block_on(cache.load(0, CancellationToken::new()))
            .unwrap()
            .catalog
            .unwrap()
            .entries()
            .is_empty()
    );
    block_on(cache.load(1000, CancellationToken::new())).unwrap();
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        block_on(cache.refresh(u64::MAX - 1, CancellationToken::new()))
            .unwrap()
            .state,
        NativeModelCatalogCacheState::Failed
    );
    block_on(cache.load(u64::MAX, CancellationToken::new())).unwrap();
    block_on(cache.load(1000, CancellationToken::new())).unwrap();
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn joining_cancel_and_drop_do_not_cancel_owner_and_owner_drop_wakes_without_stale_loading() {
    let drops = Arc::new(AtomicUsize::new(0));
    let (cache, transport) = cache([Reply::Pending(drops.clone()), Reply::Response(200, RICH)]);
    let mut owner = cache.load(0, CancellationToken::new());
    assert!(poll(owner.as_mut()).is_pending());
    let cancel = CancellationToken::new();
    let mut waiter = cache.load(0, cancel.clone());
    assert!(poll(waiter.as_mut()).is_pending());
    cancel.cancel();
    assert!(matches!(
        poll(waiter.as_mut()),
        Poll::Ready(Err(NativeModelCatalogCacheError::Cancelled))
    ));
    drop(waiter);
    let mut waiter = cache.refresh(0, CancellationToken::new());
    assert!(poll(waiter.as_mut()).is_pending());
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    drop(owner);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        block_on(waiter).unwrap().state,
        NativeModelCatalogCacheState::Idle
    );
    assert_eq!(
        block_on(cache.load(0, CancellationToken::new()))
            .unwrap()
            .state,
        NativeModelCatalogCacheState::Ready
    );
}

#[test]
fn bounded_waiter_slots_are_reused_after_drop() {
    let (cache, _) = cache([Reply::Pending(Arc::new(AtomicUsize::new(0)))]);
    let mut owner = cache.load(0, CancellationToken::new());
    assert!(poll(owner.as_mut()).is_pending());
    let mut waiters = Vec::new();
    for _ in 0..NATIVE_MODEL_CATALOG_MAX_WAITERS {
        let mut waiter = cache.wait_while_loading(CancellationToken::new());
        assert!(poll(waiter.as_mut()).is_pending());
        assert!(poll(waiter.as_mut()).is_pending());
        waiters.push(waiter);
    }
    let mut extra = cache.wait_while_loading(CancellationToken::new());
    assert!(matches!(
        poll(extra.as_mut()),
        Poll::Ready(Err(NativeModelCatalogCacheError::WaiterLimit))
    ));
    drop(waiters.pop());
    let mut replacement = cache.wait_while_loading(CancellationToken::new());
    assert!(poll(replacement.as_mut()).is_pending());
    drop(owner);
    for waiter in waiters {
        assert_eq!(
            block_on(waiter).unwrap().state,
            NativeModelCatalogCacheState::Idle
        );
    }
    assert_eq!(
        block_on(replacement).unwrap().state,
        NativeModelCatalogCacheState::Idle
    );
}

#[test]
fn cancelled_admission_is_inert_and_leader_cancellation_releases_provider() {
    let drops = Arc::new(AtomicUsize::new(0));
    let (cache, transport) = cache([Reply::Pending(drops.clone())]);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert!(matches!(
        block_on(cache.load(0, cancellation)),
        Err(NativeModelCatalogCacheError::Cancelled)
    ));
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    let cancellation = CancellationToken::new();
    let mut owner = cache.load(0, cancellation.clone());
    assert!(poll(owner.as_mut()).is_pending());
    cancellation.cancel();
    assert!(matches!(
        poll(owner.as_mut()),
        Poll::Ready(Err(NativeModelCatalogCacheError::Cancelled))
    ));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(cache.snapshot().state, NativeModelCatalogCacheState::Failed);
}

#[test]
fn completed_owner_wakes_joiners_with_shared_capabilities_and_start_timestamp() {
    let ready = Arc::new(AtomicBool::new(false));
    let (cache, transport) = cache([Reply::Gated(ready.clone())]);
    let mut owner = cache.load(1234, CancellationToken::new());
    assert!(poll(owner.as_mut()).is_pending());
    let notifications = Arc::new(AtomicUsize::new(0));
    let count = notifications.clone();
    let (waker, _) = machine_god_reentrant_waker_test::new(
        machine_god_reentrant_waker_test::Callback::Wake,
        move || {
            count.fetch_add(1, Ordering::SeqCst);
        },
    );
    let mut waiter = cache.load(9999, CancellationToken::new());
    assert!(
        waiter
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    ready.store(true, Ordering::SeqCst);
    let first = block_on(owner).unwrap();
    assert!(notifications.load(Ordering::SeqCst) > 0);
    let joined = block_on(waiter).unwrap();
    assert_eq!(joined.state, NativeModelCatalogCacheState::Ready);
    assert_eq!(joined.last_attempt_ms, Some(1234));
    assert!(Arc::ptr_eq(
        first.catalog.as_ref().unwrap(),
        joined.catalog.as_ref().unwrap()
    ));
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn waker_clone_drop_wake_callbacks_can_reenter_cache() {
    use machine_god_reentrant_waker_test::{Callback, new};
    for callback in [Callback::Clone, Callback::Drop, Callback::Wake] {
        let (cache, _) = cache([Reply::Pending(Arc::new(AtomicUsize::new(0)))]);
        let cache = Arc::new(cache);
        let weak = Arc::downgrade(&cache);
        let (waker, handle) = new(callback, move || {
            let _ = weak.upgrade().unwrap().snapshot();
        });
        let mut owner = cache.load(0, CancellationToken::new());
        assert!(poll(owner.as_mut()).is_pending());
        let mut waiter = cache.wait_while_loading(CancellationToken::new());
        for _ in 0..3 {
            assert!(
                waiter
                    .as_mut()
                    .poll(&mut Context::from_waker(&waker))
                    .is_pending()
            );
        }
        drop(owner);
        assert_eq!(
            block_on(waiter).unwrap().state,
            NativeModelCatalogCacheState::Idle
        );
        drop(waker);
        assert!(handle.calls() > 0, "{callback:?} callback was exercised");
    }
}

#[test]
fn dropping_refresh_restores_nonempty_catalog_and_previous_attempt() {
    let drops = Arc::new(AtomicUsize::new(0));
    let (cache, _) = cache([Reply::Response(200, RICH), Reply::Pending(drops.clone())]);
    let old = block_on(cache.load(10, CancellationToken::new())).unwrap();
    let mut refresh = cache.refresh(20, CancellationToken::new());
    assert!(poll(refresh.as_mut()).is_pending());
    let mut waiter = cache.wait_while_loading(CancellationToken::new());
    assert!(poll(waiter.as_mut()).is_pending());
    drop(refresh);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    let restored = block_on(waiter).unwrap();
    assert_eq!(restored.state, NativeModelCatalogCacheState::Ready);
    assert_eq!(restored.last_attempt_ms, Some(10));
    assert_eq!(restored.last_failure, old.last_failure);
    assert!(Arc::ptr_eq(
        old.catalog.as_ref().unwrap(),
        restored.catalog.as_ref().unwrap()
    ));
}
