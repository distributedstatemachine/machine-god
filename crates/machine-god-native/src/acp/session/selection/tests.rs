use super::*;
use crate::acp::selection::tests::fixture::Factory;
use crate::acp::selection::{NativeAcpSelectionOutcome, NativeAcpSelectionOwner};
use crate::mcp::ephemeral::NativeMcpEphemeralConfiguration;
use machine_god_core::{ManagedSubagentCommand, ManagedSubagentResult};
use std::sync::{Arc, atomic::Ordering};

async fn outcome(owner: &mut NativeAcpSelectionOwner) -> NativeAcpSelectionOutcome {
    std::future::poll_fn(|cx| {
        let _ = owner.poll_progress(cx, 20);
        owner.take_outcome().map_or(Poll::Pending, Poll::Ready)
    })
    .await
}

async fn command(
    owner: &mut NativeAcpSelectionOwner,
    value: serde_json::Value,
) -> ManagedSubagentResult {
    let command = ManagedSubagentCommand::decode(serde_json::json!({"command":value})).unwrap();
    let mut response = owner
        .current_mut()
        .unwrap()
        .inner
        .request_managed_command(command, CancellationToken::new())
        .unwrap();
    std::future::poll_fn(|cx| {
        let _ = owner.poll_progress(cx, 20);
        assert!(owner.current().unwrap().inner.managed_error().is_none());
        response.as_mut().poll(cx)
    })
    .await
    .unwrap()
}

#[test]
fn acp_new_and_resume_keep_the_original_child_manager_and_child_authority() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(20), async {
                let factory = Arc::new(Factory::new());
                factory.managed.store(true, Ordering::Release);
                let mut owner = NativeAcpSelectionOwner::new(factory.clone());
                let empty = || NativeMcpEphemeralConfiguration::decode(None).unwrap();
                owner
                    .request(
                        NativeAcpSessionSelection::New,
                        factory.workspace.clone(),
                        empty(),
                        1,
                    )
                    .unwrap();
                assert!(matches!(
                    outcome(&mut owner).await,
                    NativeAcpSelectionOutcome::Selected { .. }
                ));
                let original = owner.current().unwrap().id();
                let created = command(
                    &mut owner,
                    serde_json::json!({"create":{"name":"retained-acp-child","mode":"persistent"}}),
                )
                .await;
                assert!(created.ok, "{created:?}");
                let child = created.child_id.unwrap();
                for selection in [
                    NativeAcpSessionSelection::New,
                    NativeAcpSessionSelection::Resume(original),
                ] {
                    owner
                        .request(selection, factory.workspace.clone(), empty(), 2)
                        .unwrap();
                    assert!(matches!(
                        outcome(&mut owner).await,
                        NativeAcpSelectionOutcome::Selected { .. }
                    ));
                    let inspected = command(
                        &mut owner,
                        serde_json::json!({"inspect":{"id":child,"sections":["status"]}}),
                    )
                    .await;
                    assert!(inspected.ok, "{inspected:?}");
                    assert_eq!(owner.current().unwrap().inner.managed_agents().len(), 1);
                }
                assert_eq!(factory.preparations.load(Ordering::Acquire), 1);
                assert!(!factory.provider_started.load(Ordering::Acquire));
                owner.request_close(&owner.current().unwrap().id()).unwrap();
                assert!(matches!(
                    outcome(&mut owner).await,
                    NativeAcpSelectionOutcome::Closed { .. }
                ));
            })
            .await
            .expect("managed ACP replacement settles");
        });
}
