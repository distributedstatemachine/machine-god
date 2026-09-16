use super::*;
use crate::managed::manager::catalog::NativeManagedCatalogFilter;
use std::sync::atomic::AtomicUsize;

#[test]
fn observation_waits_for_original_journal_work_and_shutdown_drains_it() {
    let mut fixture = Fixture::new(vec![]);
    let (old_sender, old_receiver) = tokio::sync::oneshot::channel();
    fixture.manager.active = Some(Active::Observation(Box::pin(async move {
        old_receiver.await.unwrap();
    })));
    assert!(
        fixture
            .manager
            .request_observation(Box::pin(async {}))
            .is_err()
    );
    old_sender.send(()).unwrap();
    fixture.drive(|fixture| fixture.manager.active.is_none());

    let request = fixture
        .manager
        .request_catalog(NativeManagedCatalogFilter::All, None, 1)
        .unwrap();
    assert!(fixture.manager.begin_catalog());
    let Some(Active::Catalog { future, .. }) = &mut fixture.manager.active else {
        panic!("catalog owns journal admission");
    };
    let (catalog_sender, catalog_receiver) = tokio::sync::oneshot::channel();
    *future = Box::pin(async move { catalog_receiver.await.unwrap() });
    let started = Arc::new(AtomicUsize::new(0));
    let observed = started.clone();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    fixture
        .manager
        .request_observation(Box::pin(async move {
            observed.fetch_add(1, Ordering::SeqCst);
            receiver.await.unwrap();
        }))
        .unwrap();
    assert!(
        fixture
            .manager
            .request_observation(Box::pin(async {}))
            .is_err()
    );
    let mut cx = Context::from_waker(Waker::noop());
    let _ = fixture.manager.poll_progress(&mut cx, 100);
    assert_eq!(started.load(Ordering::SeqCst), 0);
    fixture.manager.request_shutdown();
    assert!(fixture.manager.poll_shutdown(&mut cx, 100).is_pending());
    catalog_sender
        .send(Ok(store::JournalCatalogPage {
            entries: vec![],
            next: None,
        }))
        .unwrap();
    fixture.drive(|_| started.load(Ordering::SeqCst) == 1);
    assert_eq!(
        fixture.manager.take_catalog_outcome().unwrap().request,
        request
    );
    assert!(fixture.manager.poll_shutdown(&mut cx, 100).is_pending());
    sender.send(()).unwrap();
    block_on(std::future::poll_fn(|cx| {
        fixture.manager.poll_shutdown(cx, 100)
    }))
    .unwrap();
    assert!(
        fixture
            .manager
            .request_observation(Box::pin(async {}))
            .is_err()
    );
}

#[test]
fn repeated_observations_yield_to_queued_durable_work_with_one_step_budget() {
    let mut fixture = Fixture::new(vec![]);
    fixture.manager.limits.work_per_poll = 1;
    let (_admission, invocation) = fixture.invocation(serde_json::json!({
        "create":{"name":"must-progress","mode":"persistent"}
    }));
    let requester = fixture.requester.clone();
    let mut response = requester.execute(invocation, CancellationToken::new());
    let reads = Arc::new(AtomicUsize::new(0));
    let result = block_on(std::future::poll_fn(|cx| {
        if fixture.manager.observation.is_none()
            && !matches!(fixture.manager.active, Some(Active::Observation(_)))
        {
            let reads = reads.clone();
            fixture
                .manager
                .request_observation(Box::pin(async move {
                    reads.fetch_add(1, Ordering::SeqCst);
                }))
                .unwrap();
        }
        if let Poll::Ready(result) = response.as_mut().poll(cx) {
            return Poll::Ready(result.unwrap());
        }
        let progress = fixture.manager.poll_progress(cx, 100);
        assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
        assert!(
            reads.load(Ordering::SeqCst) <= 2,
            "observation starved command"
        );
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    }));
    assert!(result.ok, "{result:?}");
    assert!(reads.load(Ordering::SeqCst) >= 1);
}
