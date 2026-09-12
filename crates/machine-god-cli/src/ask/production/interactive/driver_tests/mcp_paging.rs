use super::super::super::mcp_test_support as http;
use super::*;
use native::{NativeInteractiveControlReceipt, mcp::control::McpFeatureReply};
use serde_json::json;

#[test]
fn actual_feature_owner_and_page_cursor_survive_blocked_output_and_shutdown() {
    let runtime = executor();
    let selected = runtime.block_on(http::setup());
    let fixture = selected.fixture;
    let listener = selected.listener;
    let mut harness = runtime.block_on(harness_with_prompts(
        &fixture,
        selected.bridge,
        selected.inbox,
    ));
    harness.driver.notice.take();
    harness
        .driver
        .command("/mcp resource read fixture test://fixed", 100);
    let outcome = runtime.block_on(fetch_resource(&mut harness, &listener));
    let id = outcome.id;
    let Ok(NativeInteractiveControlReceipt::McpFeature(receipt)) = &outcome.result else {
        panic!("actual feature receipt")
    };
    assert!(receipt.revalidate().is_ok());
    let McpFeatureReply::Response(response) = receipt.reply() else {
        panic!("resource response")
    };
    assert!(response.result_json().get().len() > 24_000);
    let frame = harness
        .driver
        .control_page
        .render(id.get(), receipt)
        .unwrap();
    assert!(
        harness
            .driver
            .control_page
            .render(id.get(), receipt)
            .is_err()
    );
    harness.driver.control_outcome = Some(outcome);
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
        assert_feature(harness.driver.control_outcome.as_ref().unwrap(), id);
        assert!(fixture.transport.requests().is_empty());
        harness.driver.shutdown();
        until(&mut harness, |driver| driver.owner.is_closed()).await;
        fixture.host.close_mcp();
        let cleanup = fixture
            .host
            .drain_mcp(http::deadline(), CancellationToken::new())
            .await
            .unwrap();
        assert!(
            cleanup
                .iter()
                .all(native::mcp::runtime::NativeMcpPeerCompletion::is_complete)
        );
        poll_fn(|cx| harness.driver.poll(cx, &mut harness.signals)).await
    });
    let mut tail = dispose(harness, fixture, result);
    let retained = tail.presentation.controls.front().unwrap();
    assert_feature(retained, id);
    let Ok(NativeInteractiveControlReceipt::McpFeature(receipt)) = &retained.result else {
        unreachable!()
    };
    assert!(
        receipt.revalidate().is_err(),
        "closed authority cannot be revived by retained data"
    );
    assert!(
        tail.presentation
            .control_page
            .render(id.get(), receipt)
            .is_err()
    );
    runtime.block_on(drain_page(&mut tail, &frame));
    let listener = listener.into_std().unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

async fn fetch_resource(
    harness: &mut Harness,
    listener: &tokio::net::TcpListener,
) -> native::NativeInteractiveControlOutcome {
    let (outcome, ()) = tokio::time::timeout(
        Duration::from_secs(10),
        futures_util::future::join(
            poll_fn(|cx| {
                let _ = harness.driver.owner.poll_progress(cx, 100);
                harness
                    .driver
                    .owner
                    .take_control_outcome()
                    .map_or(Poll::Pending, Poll::Ready)
            }),
            async {
                http::reply(listener, "resources/list", http::resources()).await;
                http::reply(
                    listener,
                    "resources/read",
                    json!({"contents":[{"uri":"test://fixed","text":"🦀".repeat(6000)}]}),
                )
                .await;
            },
        ),
    )
    .await
    .unwrap();
    outcome
}

fn assert_feature(
    outcome: &native::NativeInteractiveControlOutcome,
    id: native::NativeInteractiveControlId,
) {
    assert_eq!(outcome.id, id);
    assert!(matches!(
        &outcome.result,
        Ok(NativeInteractiveControlReceipt::McpFeature(_))
    ));
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
                assert_feature(tail.presentation.controls.front().unwrap(), id);
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
                        assert_feature(tail.presentation.controls.front().unwrap(), id);
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
