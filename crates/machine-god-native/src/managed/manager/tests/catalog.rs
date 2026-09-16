use super::super::catalog::{NativeManagedCatalogError, NativeManagedCatalogFilter};
use super::*;

#[test]
fn repeated_catalog_refresh_yields_to_queued_durable_work_with_one_step_budget() {
    let mut fixture = Fixture::new(vec![]);
    fixture.manager.limits.work_per_poll = 1;
    let (_admission, invocation) = fixture.invocation(serde_json::json!({
        "create":{"name":"must-progress","mode":"persistent"}
    }));
    let requester = fixture.requester.clone();
    let mut response = requester.execute(invocation, CancellationToken::new());
    let mut pending_read = None;
    let mut completed_reads = 0;
    let result = block_on(std::future::poll_fn(|cx| {
        // Keep the catalog continuously requested, including before each
        // manager admission, while preserving the original command future.
        if pending_read.is_none() {
            pending_read = Some(
                fixture
                    .manager
                    .request_catalog(NativeManagedCatalogFilter::All, None, 1)
                    .unwrap(),
            );
        }
        if let Poll::Ready(result) = response.as_mut().poll(cx) {
            return Poll::Ready(result.unwrap());
        }
        let progress = fixture.manager.poll_progress(cx, 100);
        assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
        if let Some(outcome) = fixture.manager.take_catalog_outcome() {
            assert_eq!(Some(outcome.request), pending_read.take());
            assert!(outcome.result.is_ok());
            completed_reads += 1;
            assert!(
                completed_reads <= 2,
                "catalog refresh starved the original command"
            );
            cx.waker().wake_by_ref();
        }
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    }));
    assert!(result.ok, "{result:?}");
    assert!(completed_reads >= 1);
}

#[test]
fn shutdown_retains_the_original_inflight_catalog_until_it_resolves() {
    let mut fixture = Fixture::new(vec![]);
    let request = fixture
        .manager
        .request_catalog(NativeManagedCatalogFilter::All, None, 1)
        .unwrap();
    assert!(fixture.manager.begin_catalog());
    let Some(Active::Catalog { future, .. }) = &mut fixture.manager.active else {
        panic!("original catalog operation");
    };
    let (sender, receiver) = tokio::sync::oneshot::channel();
    *future = Box::pin(async move { receiver.await.unwrap() });
    fixture.manager.request_shutdown();
    assert!(fixture.manager.take_catalog_outcome().is_none());
    let mut cx = Context::from_waker(Waker::noop());
    assert!(fixture.manager.poll_shutdown(&mut cx, 100).is_pending());
    assert!(matches!(
        fixture.manager.active,
        Some(Active::Catalog { .. })
    ));
    sender
        .send(Ok(crate::managed::store::JournalCatalogPage {
            entries: vec![],
            next: None,
        }))
        .unwrap();
    block_on(std::future::poll_fn(|cx| {
        fixture.manager.poll_shutdown(cx, 100)
    }))
    .unwrap();
    let outcome = fixture.manager.take_catalog_outcome().unwrap();
    assert_eq!(outcome.request, request);
    assert!(matches!(
        outcome.result,
        Err(NativeManagedCatalogError::Closed)
    ));
}
