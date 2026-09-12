use super::*;
use native::mcp::{
    catalog::{McpDescriptorCatalog, McpDescriptorLimits},
    control::McpFeatureReply,
    pagination::{McpCatalogBuilder, McpCatalogKind, McpCatalogLimits},
    protocol::{ProtocolVersion, RpcId},
};

fn large_catalog() -> McpFeatureReply {
    let wire = format!(
        r#"{{"jsonrpc":"2.0","id":1,"result":{{"resources":[{{"uri":"test://fixed","name":"fixed","_meta":{{"padding":"{}"}}}}]}}}}"#,
        "x".repeat(20_000)
    );
    let mut builder = McpCatalogBuilder::new(
        McpCatalogKind::Resources,
        ProtocolVersion::Modern,
        McpCatalogLimits::default(),
    )
    .unwrap();
    builder
        .append_response(wire.as_bytes(), &RpcId::Integer(1), None, 0)
        .unwrap();
    McpFeatureReply::Catalog(
        McpDescriptorCatalog::admit(builder.finish().unwrap(), McpDescriptorLimits::default())
            .unwrap(),
    )
}

#[test]
fn pending_page_cursor_and_control_owner_survive_blocked_output_and_shutdown() {
    let runtime = executor();
    let fixture = support::Fixture::new_with_mcp();
    let mut harness = runtime.block_on(harness(&fixture));
    harness.driver.notice.take();
    harness.driver.command("/mcp", 100);
    let outcome = runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                let _ = harness.driver.owner.poll_progress(cx, 100);
                harness
                    .driver
                    .owner
                    .take_control_outcome()
                    .map_or(Poll::Pending, Poll::Ready)
            }),
        )
        .await
        .unwrap()
    });
    let id = outcome.id;
    harness.driver.control_outcome = Some(outcome);
    // The ordinary native outcome exercises the shared acknowledgement owner;
    // typed feature data and exact reconstruction are tested in the pager suite.
    let reply = large_catalog();
    let frame = harness
        .driver
        .control_page
        .prepare(
            id.get(),
            native::McpFeatureAction::ResourceList,
            "srv",
            &reply,
            true,
        )
        .unwrap();
    harness.driver.render = Some(Render {
        history: false,
        clear_row: false,
        bytes: frame.clone(),
        offset: 0,
        confirm: None,
        receipt: Some(ReceiptKind::Control),
        model_text: false,
    });
    let result = runtime.block_on(async {
        until(&mut harness, |driver| driver.in_flight.is_some()).await;
        assert!(harness.driver.control_outcome.is_some());
        assert!(
            harness
                .driver
                .control_page
                .prepare(
                    id.get(),
                    native::McpFeatureAction::ResourceList,
                    "srv",
                    &reply,
                    true
                )
                .is_err()
        );
        assert!(fixture.transport.requests().is_empty());
        harness.driver.shutdown();
        until(&mut harness, |driver| driver.owner.is_closed()).await;
        poll_fn(|cx| harness.driver.poll(cx, &mut harness.signals)).await
    });
    let mut tail = dispose(harness, fixture, result);
    assert_eq!(tail.presentation.controls.front().unwrap().id, id);
    assert!(
        tail.presentation
            .control_page
            .prepare(
                id.get(),
                native::McpFeatureAction::ResourceList,
                "srv",
                &reply,
                true
            )
            .is_err()
    );
    runtime.block_on(drain_page(&mut tail, &frame));
}

async fn drain_page(tail: &mut TailHarness, frame: &[u8]) {
    let id = tail.presentation.controls.front().unwrap().id;
    let mut bytes = Vec::new();
    let mut page_flush = false;
    let mut check_retained = false;
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            let result = tail.presentation.poll(cx, &mut tail.signals);
            if check_retained {
                assert_eq!(
                    tail.presentation.controls.front().unwrap().id,
                    id,
                    "acknowledging a non-final page must not release the original outcome"
                );
                check_retained = false;
            }
            if let Ok(work) = tail.work.try_recv() {
                match work {
                    OutputWork::Write(chunk) => bytes.extend(chunk),
                    OutputWork::Flush if !page_flush => {
                        assert_eq!(
                            bytes, frame,
                            "split writes must neither repeat nor skip accepted page bytes"
                        );
                        assert_eq!(tail.presentation.controls.front().unwrap().id, id);
                        page_flush = true;
                        check_retained = true;
                    }
                    OutputWork::Flush => {}
                }
                tail.ack.try_send(OutputAcknowledgement::Succeeded).unwrap();
                cx.waker().wake_by_ref();
            }
            result
        }),
    )
    .await
    .unwrap();
    assert!(page_flush);
    assert!(tail.presentation.controls.is_empty());
}
