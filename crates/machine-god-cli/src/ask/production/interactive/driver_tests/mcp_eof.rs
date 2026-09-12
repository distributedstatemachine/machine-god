//! Physical pipe EOF is separate from canonical modal cancel and PTY hangup.
use super::super::super::{mcp_test_support as http, presentation::Modal};
use super::*;
use serde_json::json;

#[test]
fn physical_pipe_eof_retires_pending_mcp_human_command_without_an_answer() {
    for raw in [false, true] {
        let runtime = executor();
        let selected = runtime.block_on(http::setup());
        let fixture = selected.fixture;
        let listener = selected.listener;
        let mut harness = runtime.block_on(harness_with_prompts(
            &fixture,
            selected.bridge,
            selected.inbox,
        ));
        if raw {
            harness.driver = harness.driver.with_raw_input(80, None);
        }
        harness.driver.notice.take();
        let mut modal = runtime.block_on(begin(&mut harness, &listener));
        let token = modal.view.token().clone();
        modal.displayed = true;
        assert!(modal.answer("/next", &modal.binding()).unwrap().is_none());
        modal.displayed = true;
        let stale = modal.answer("y", &modal.binding()).unwrap().unwrap();
        harness.driver.modal = Some(modal);
        let (_, replacement) = std::io::pipe().unwrap();
        drop(std::mem::replace(&mut harness.input_writer, replacement));
        let result = runtime.block_on(drive_to_eof(&mut harness));
        assert_eq!(
            result.outcome,
            if raw {
                AskCommandOutcome::OperationalFailure
            } else {
                AskCommandOutcome::Completed
            }
        );
        assert!(harness.driver.owner.is_closed());
        assert!(harness.driver.inbox.reply(&token, stale).is_err());
        assert!(fixture.transport.requests().is_empty());
        fixture.host.close_mcp();
        let cleanup = runtime
            .block_on(
                fixture
                    .host
                    .drain_mcp(http::deadline(), CancellationToken::new()),
            )
            .unwrap();
        assert!(
            cleanup
                .iter()
                .all(native::mcp::runtime::NativeMcpPeerCompletion::is_complete)
        );
        let mut tail = dispose(harness, fixture, result);
        let (settled, _) = runtime.block_on(finish_raw_tail(&mut tail));
        assert_eq!(settled.outcome, result.outcome);
        assert_eq!(
            listener.into_std().unwrap().accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "physical EOF must not manufacture a canonical cancel or accept round"
        );
    }
}

async fn begin(harness: &mut Harness, listener: &tokio::net::TcpListener) -> Modal {
    harness
        .driver
        .command("/mcp resource read fixture test://fixed", 100);
    let (modal, ()) = tokio::time::timeout(
        Duration::from_secs(10),
        futures_util::future::join(
            poll_fn(|cx| {
                let _ = harness.driver.owner.poll_progress(cx, 101);
                assert!(harness.driver.owner.take_control_outcome().is_none());
                harness
                    .driver
                    .inbox
                    .poll_prompt(cx)
                    .map(|view| Modal::new(view.expect("pending actual MCP elicitation")))
            }),
            async {
                http::reply(listener, "resources/list", http::resources()).await;
                http::reply(
                    listener,
                    "resources/read",
                    json!({
                        "resultType":"input_required", "requestState":null,
                        "inputRequests":{"confirm":{"method":"elicitation/create","params":{
                            "message":"Confirm exact command", "requestedSchema":{
                                "type":"object", "properties":{}
                            }
                        }}}
                    }),
                )
                .await;
            },
        ),
    )
    .await
    .unwrap();
    modal
}

async fn drive_to_eof(harness: &mut Harness) -> TurnDriveResult {
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            let result = harness.driver.poll(cx, &mut harness.signals);
            if harness.work.try_recv().is_ok() {
                harness
                    .ack
                    .try_send(OutputAcknowledgement::Succeeded)
                    .unwrap();
                cx.waker().wake_by_ref();
            }
            result
        }),
    )
    .await
    .unwrap()
}
