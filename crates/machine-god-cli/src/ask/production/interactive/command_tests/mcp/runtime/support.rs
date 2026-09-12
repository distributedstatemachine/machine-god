//! Command-specific driver assembly over the shared actual HTTP fixture.
pub(super) use super::super::super::super::super::mcp_test_support::{
    deadline, prompts, reply, resources, templates,
};
use super::*;

pub(super) async fn setup() -> (support::Fixture, Driver, TcpListener) {
    let selected = super::super::super::super::super::mcp_test_support::setup().await;
    let driver = super::super::super::driver_with_inbox(&selected.fixture, selected.inbox).await;
    (selected.fixture, driver, selected.listener)
}

pub(super) async fn setup_with_host_options(
    select: impl FnOnce(
        native::NativeReferenceHostConversationOptions,
    ) -> native::NativeReferenceHostConversationOptions,
) -> (support::Fixture, Driver, TcpListener) {
    let selected =
        super::super::super::super::super::mcp_test_support::setup_with_host_options(select).await;
    let driver = super::super::super::driver_with_inbox(&selected.fixture, selected.inbox).await;
    (selected.fixture, driver, selected.listener)
}

pub(super) async fn finish_runtime(driver: Driver, fixture: support::Fixture) {
    fixture.host.close_mcp();
    let receipts = fixture
        .host
        .drain_mcp(deadline(), CancellationToken::new())
        .await
        .unwrap();
    assert!(
        receipts
            .iter()
            .all(machine_god_native::mcp::runtime::NativeMcpPeerCompletion::is_complete)
    );
    Box::pin(super::super::super::finish(driver, fixture)).await;
}
