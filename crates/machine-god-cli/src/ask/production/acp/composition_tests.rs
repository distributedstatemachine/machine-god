//! Composed CLI coverage, not a production-executable or live-Gateway claim.
//! Only captured launch/input authorities and the two network endpoints differ.

mod gateway;
mod support;

use super::{AskCommandOutcome, AskSignalController, CapturedAcpLaunch, execute_with_capture};
use machine_god_core::{Message, Role, SessionId, SessionStore};
use machine_god_native::{FileSessionStore, NativeSessionMetadata, NativeSessionOrigin};
use serde_json::json;
use std::time::{Duration, Instant};
use support::{Client, Fixture};

const PROMPT: &str = "complete the composed ACP roundtrip";
const ANSWER: &str = "local fixture answer";

#[test]
fn shared_acquisition_owned_stdio_and_native_prompt_checkpoint_roundtrip() {
    let fixture = Fixture::new();
    let gateway = gateway::Gateway::new();
    let helper = support::release_helper();
    let launch = CapturedAcpLaunch::loopback(
        fixture.environment(),
        helper.clone(),
        "/bin/bash".into(),
        gateway.address,
    )
    .unwrap();
    let (input, writer) = std::io::pipe().unwrap();
    let (reader, mut output) = std::io::pipe().unwrap();
    let source = support::input_source(input, &helper);
    let mut client = Client::new(reader, writer, Instant::now() + Duration::from_secs(10));

    std::thread::scope(|scope| {
        let worker = scope.spawn(move || {
            let mut controller = AskSignalController::spawn().unwrap();
            // Keep the exact guardian join even if the composed call unwinds.
            let guardian = support::JoinedGuardian(controller.worker.take());
            let (outcome, controller) =
                execute_with_capture(&mut output, controller, || Ok(source), || Ok(launch));
            drop(controller);
            drop(guardian);
            outcome
        });
        let scenario = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.send(1, "initialize", json!({"protocolVersion":1}));
            assert_eq!(client.response(1).0["protocolVersion"], 1);
            assert!(gateway.requests().is_empty());
            assert!(!fixture.session_root().exists());

            client.send(
                2,
                "session/new",
                json!({"cwd":fixture.workspace,"mcpServers":[]}),
            );
            let selected = client.response(2).0;
            let id = selected["sessionId"]
                .as_str()
                .expect("native session identity")
                .to_owned();
            assert_eq!(
                gateway
                    .requests()
                    .iter()
                    .map(|r| r.method.as_str())
                    .collect::<Vec<_>>(),
                ["catalog"]
            );

            client.send(
                3,
                "session/prompt",
                json!({"sessionId":id,
                "prompt":[{"type":"text","text":PROMPT}]}),
            );
            let (completed, updates) = client.response(3);
            assert_eq!(completed["stopReason"], "end_turn");
            let text: String = updates
                .iter()
                .filter(|u| u["update"]["sessionUpdate"] == "agent_message_chunk")
                .filter_map(|u| u["update"]["content"]["text"].as_str())
                .collect();
            assert_eq!(
                text, ANSWER,
                "assistant presentation precedes final RPC response"
            );

            let requests = gateway.requests();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[1].method, "inference");
            assert!(requests.iter().all(|r| r.authorized));
            assert!(requests[1].body.to_string().contains(PROMPT));

            // Read durable state immediately after the final wire response,
            // while this exact session is still selected and stdin stays open.
            assert_checkpoint(&fixture, SessionId::new(id).unwrap());
            fixture.assert_profile_unchanged();
            client.eof();
            client.drain_to_eof();
        }));
        // On assertion/deadline failure, release both pipes before joining the
        // producer. No reader, output writer, or guardian is detached.
        drop(client);
        let outcome = worker.join().unwrap();
        if let Err(payload) = scenario {
            std::panic::resume_unwind(payload);
        }
        assert_eq!(outcome, AskCommandOutcome::Completed);
    });
    gateway.finish();
}

fn assert_checkpoint(fixture: &Fixture, id: SessionId) {
    let store = FileSessionStore::open(&fixture.session_root()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let record = runtime.block_on(store.load(id)).unwrap().unwrap();
    assert_eq!(
        record.messages,
        [
            Message::text(Role::User, PROMPT),
            Message::text(Role::Assistant, ANSWER)
        ]
    );
    let metadata = NativeSessionMetadata::from_metadata(&record.metadata).unwrap();
    assert_eq!(metadata.origin(), Some(NativeSessionOrigin::Acp));
    assert_eq!(metadata.workspace(), Some(fixture.workspace.as_path()));
}
