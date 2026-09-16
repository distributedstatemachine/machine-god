use super::*;
use machine_god_core::BoxFuture;
use native::{
    AiGatewayModelCatalogAccessMode, AiGatewayModelCatalogProvider,
    AiGatewayModelCatalogRequestAccess, AiGatewayModelCatalogTransport,
    AiGatewayModelCatalogTransportError, AiGatewayModelCatalogTransportResponse,
    NativeModelCatalogCache, NativeModelCatalogCacheState,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Default)]
struct Catalog {
    ready: AtomicBool,
    calls: AtomicUsize,
    wake: futures_util::task::AtomicWaker,
}
impl AiGatewayModelCatalogTransport for Catalog {
    fn get(
        &self,
        _: AiGatewayModelCatalogRequestAccess,
        _: std::time::Instant,
        cancellation: CancellationToken,
    ) -> BoxFuture<
        '_,
        Result<AiGatewayModelCatalogTransportResponse, AiGatewayModelCatalogTransportError>,
    > {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut cancelled = std::pin::pin!(cancellation.cancelled());
            poll_fn(|cx| {
                self.wake.register(cx.waker());
                if cancelled.as_mut().poll(cx).is_ready() || self.ready.load(Ordering::SeqCst) {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            })
            .await;
            Ok(AiGatewayModelCatalogTransportResponse::new(200, br#"{"data":[{"id":"vendor/first","released":2},{"id":"vendor/second","released":1}]}"#.to_vec()))
        })
    }
    fn wait_until(&self, _: std::time::Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}
fn cache() -> (Arc<Catalog>, Arc<NativeModelCatalogCache>) {
    let transport = Arc::new(Catalog::default());
    let cache = Arc::new(NativeModelCatalogCache::new(Arc::new(
        AiGatewayModelCatalogProvider::new(
            AiGatewayModelCatalogAccessMode::PublicOnly,
            transport.clone(),
        ),
    )));
    (transport, cache)
}
fn models_displayed(driver: &Driver) -> bool {
    displayed(driver) && driver.owner.managed_navigation().unwrap().route == Route::Models
}
fn menu(driver: &Driver) -> native::NativeManagedModelsView<'_> {
    driver.owner.managed_navigation().unwrap().models.unwrap()
}

async fn assert_saved_model(harness: &mut Harness) {
    harness.input_writer.write_all(b"/configuration\r").unwrap();
    pump_until(harness, |driver| {
        displayed(driver)
            && driver.owner.managed_navigation().unwrap().route
                == Route::Agent(machine_god_core::ManagedInspectSection::Configuration)
    })
    .await;
    let view = harness.driver.owner.managed_navigation().unwrap();
    let machine_god_core::ManagedRequested::Inspection(inspection) =
        view.result.unwrap().requested.as_ref().unwrap()
    else {
        panic!("configuration inspection required")
    };
    assert_eq!(
        inspection.configuration.as_ref().unwrap().model.as_deref(),
        Some("vendor/second")
    );
}

async fn reject_stale_query(harness: &mut Harness) {
    let old = harness.driver.owner.managed_navigation().unwrap().frame;
    harness.input_writer.write_all(b"unmatchable").unwrap();
    pump_until(harness, |driver| {
        models_displayed(driver) && menu(driver).picker.query == "unmatchable"
    })
    .await;
    assert!(menu(&harness.driver).picker.rows().next().is_none());
    assert_eq!(
        harness
            .driver
            .owner
            .act_on_managed_frame(&old, native::NativeManagedNavigationAction::Select),
        Err(native::NativeManagedNavigationError::StaleFrame)
    );
    harness.input_writer.write_all(b"\x1b").unwrap();
    pump_until(harness, |driver| {
        displayed(driver) && driver.owner.managed_navigation().unwrap().route == Route::Conversation
    })
    .await;
}

#[test]
fn model_loading_survives_parent_return_and_selection_configures_only_the_child() {
    let runtime = executor();
    let (transport, cache) = cache();
    let (fixture, mut harness) = runtime.block_on(prepared_with_catalog(Some(cache.clone())));
    let result = runtime.block_on(async {
        harness.input_writer.write_all(b"parent draft").unwrap();
        pump_until(&mut harness, |driver| {
            driver
                .input
                .raw_draft()
                .is_some_and(|(text, _)| text == "parent draft")
        })
        .await;
        let parent = harness.driver.owner.runtime().clone();
        let parent_metadata = parent.record().metadata.clone();
        enter_child(&mut harness).await;
        harness.input_writer.write_all(b"/models\r").unwrap();
        pump_until(&mut harness, models_displayed).await;
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        harness.input_writer.write_all(b"\x18").unwrap();
        pump_until(&mut harness, |driver| {
            driver.agents.is_none() && presentation_idle(driver)
        })
        .await;
        assert_eq!(harness.driver.input.raw_draft().unwrap().0, "parent draft");
        transport.ready.store(true, Ordering::SeqCst);
        transport.wake.wake();
        pump_until(&mut harness, |_| {
            cache.snapshot().state == NativeModelCatalogCacheState::Ready
        })
        .await;
        enter_child(&mut harness).await;
        harness.input_writer.write_all(b"/models\r").unwrap();
        pump_until(&mut harness, |driver| {
            models_displayed(driver) && menu(driver).state == NativeModelCatalogCacheState::Ready
        })
        .await;
        reject_stale_query(&mut harness).await;
        harness.input_writer.write_all(b"/models\r").unwrap();
        pump_until(&mut harness, models_displayed).await;
        harness.input_writer.write_all(b"\x0a").unwrap();
        pump_until(&mut harness, |driver| {
            models_displayed(driver) && menu(driver).picker.selected == Some(1)
        })
        .await;
        harness.input_writer.write_all(b"\r").unwrap();
        pump_until(&mut harness, |driver| {
            displayed(driver) && driver.owner.managed_navigation().unwrap().form.is_some()
        })
        .await;
        assert!(
            harness
                .driver
                .owner
                .managed_navigation()
                .unwrap()
                .form
                .unwrap()
                .values
                .contains(&"vendor/second")
        );
        harness.input_writer.write_all(b"\r").unwrap();
        pump_until(&mut harness, |driver| {
            displayed(driver) && driver.owner.managed_navigation().unwrap().form.is_none()
        })
        .await;
        assert!(
            harness
                .driver
                .owner
                .managed_navigation()
                .unwrap()
                .result
                .unwrap()
                .ok
        );
        assert_saved_model(&mut harness).await;
        assert!(Arc::ptr_eq(&parent, harness.driver.owner.runtime()));
        assert_eq!(parent.record().metadata, parent_metadata);
        assert!(Arc::ptr_eq(
            &parent.model_catalog().unwrap(),
            &cache.snapshot().catalog.unwrap()
        ));
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        assert!(fixture.transport.requests().is_empty());
        drop(parent);
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn shutdown_cancels_an_owned_catalog_load_without_a_model_turn() {
    let runtime = executor();
    let (transport, cache) = cache();
    let (fixture, mut harness) = runtime.block_on(prepared_with_catalog(Some(cache)));
    let result = runtime.block_on(async {
        enter_child(&mut harness).await;
        harness.input_writer.write_all(b"/models\r").unwrap();
        pump_until(&mut harness, models_displayed).await;
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        assert!(fixture.transport.requests().is_empty());
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}
