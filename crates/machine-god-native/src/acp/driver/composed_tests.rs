//! Wire dispatch through real native persistence, selection and provider owners.
use super::*;
use crate::acp::{protocol::decode_frame, selection::tests::fixture::Factory};
use serde_json::json;
use std::sync::atomic::Ordering;

fn run(future: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(20), future)
                .await
                .expect("composed ACP connection must settle");
        });
}
fn request(connection: &mut NativeAcpConnection, id: i64, method: &str, params: Value) {
    connection
        .receive(
            AcpMessage::Request {
                id: AcpId::Integer(id),
                method: method.to_owned(),
                params: Some(params),
            },
            200,
        )
        .unwrap();
}
async fn next(connection: &mut NativeAcpConnection) -> AcpMessage {
    let bytes = futures_util::future::poll_fn(|cx| connection.poll_output(cx, 200))
        .await
        .expect("connection must retain its reply");
    decode_frame(&bytes).unwrap()
}
async fn response(
    connection: &mut NativeAcpConnection,
    id: i64,
) -> (Result<Value, AcpRpcError>, Vec<Value>) {
    let mut updates = Vec::new();
    loop {
        assert!(updates.len() < 128, "bounded fixture output");
        match next(connection).await {
            AcpMessage::Response {
                id: Some(actual),
                outcome,
            } => {
                assert_eq!(actual, AcpId::Integer(id));
                return (outcome, updates);
            }
            AcpMessage::Notification {
                method,
                params: Some(params),
            } => {
                assert_eq!(method, "session/update");
                updates.push(params);
            }
            message => panic!("unexpected native fixture frame: {message:?}"),
        }
    }
}
async fn connection(factory: Arc<Factory>) -> (NativeAcpConnection, String) {
    let mut connection =
        NativeAcpConnection::new(factory.clone(), NativeAcpClientRequests::new().unwrap());
    request(
        &mut connection,
        1,
        "initialize",
        json!({"protocolVersion":1}),
    );
    response(&mut connection, 1).await.0.unwrap();
    request(
        &mut connection,
        2,
        "session/new",
        json!({"cwd":factory.workspace}),
    );
    let (result, _) = response(&mut connection, 2).await;
    let id = result.unwrap()["sessionId"].as_str().unwrap().to_owned();
    (connection, id)
}
async fn started(connection: &mut NativeAcpConnection, factory: &Factory) {
    futures_util::future::poll_fn(|cx| {
        // Consume normal startup presentation until the provider is running.
        // The EOF scenarios stop consuming output only after this checkpoint.
        if let Poll::Ready(frame) = connection.poll_output(cx, 200) {
            assert!(matches!(
                decode_frame(&frame.expect("live connection")).unwrap(),
                AcpMessage::Notification { .. }
            ));
            cx.waker().wake_by_ref();
        }
        if factory.provider_started.load(Ordering::Acquire) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
}
fn cancel(connection: &mut NativeAcpConnection, id: &str) {
    connection
        .receive(
            AcpMessage::Notification {
                method: "session/cancel".to_owned(),
                params: Some(json!({"sessionId":id})),
            },
            200,
        )
        .unwrap();
}
async fn shutdown(connection: &mut NativeAcpConnection) {
    connection.begin_shutdown();
    futures_util::future::poll_fn(|cx| {
        let _ = connection.poll_progress(cx, 200);
        if connection.is_closed() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
    assert!(connection.error().is_none());
}

#[test]
fn prompt_cancel_load_and_resume_follow_native_checkpoint_receipts() {
    run(async {
        let factory = Arc::new(Factory::new());
        let (mut connection, id) = connection(factory.clone()).await;
        request(
            &mut connection,
            3,
            "session/prompt",
            json!({"sessionId":id,
            "prompt":[{"type":"text","text":"canonical persisted input"}]}),
        );
        started(&mut connection, &factory).await;
        cancel(&mut connection, &id);
        let (result, _) = response(&mut connection, 3).await;
        assert_eq!(result.unwrap()["stopReason"], "cancelled");
        factory.provider_started.store(false, Ordering::Release);
        request(
            &mut connection,
            4,
            "session/load",
            json!({"sessionId":id,"cwd":factory.workspace}),
        );
        let (result, updates) = response(&mut connection, 4).await;
        assert_eq!(result.unwrap()["sessionId"], id);
        assert!(
            updates.iter().any(
                |params| params["update"]["sessionUpdate"] == "user_message_chunk"
                    && params["update"]["content"]["text"] == "canonical persisted input"
            )
        );
        assert!(!factory.provider_started.load(Ordering::Acquire));
        request(
            &mut connection,
            5,
            "session/resume",
            json!({"sessionId":id,"cwd":factory.workspace}),
        );
        let (result, updates) = response(&mut connection, 5).await;
        assert_eq!(result.unwrap()["sessionId"], id);
        assert!(
            !updates
                .iter()
                .any(|params| params["update"]["sessionUpdate"] == "user_message_chunk")
        );
        assert!(!factory.provider_started.load(Ordering::Acquire));
        shutdown(&mut connection).await;
    });
}

#[test]
fn failed_required_mcp_candidate_keeps_old_prompt_and_selection_usable() {
    run(async {
        let factory = Arc::new(Factory::new());
        let (mut connection, id) = connection(factory.clone()).await;
        let previous = connection.selection.current().unwrap().principal();
        request(
            &mut connection,
            3,
            "session/prompt",
            json!({"sessionId":id,
            "prompt":[{"type":"text","text":"keep this running"}]}),
        );
        started(&mut connection, &factory).await;
        request(
            &mut connection,
            4,
            "session/new",
            json!({"cwd":factory.workspace,
            "mcpServers":[{"name":"unavailable","command":"/never/selected/helper","args":[],"env":[]}]}),
        );
        assert!(response(&mut connection, 4).await.0.is_err());
        assert_eq!(
            connection.selection.current().unwrap().principal(),
            previous
        );
        assert!(connection.prompt.is_some());
        cancel(&mut connection, &id);
        assert_eq!(
            response(&mut connection, 3).await.0.unwrap()["stopReason"],
            "cancelled"
        );
        shutdown(&mut connection).await;
    });
}

#[test]
fn eof_finalizes_running_native_prompt_without_an_output_consumer() {
    run(async {
        let factory = Arc::new(Factory::new());
        let (mut connection, id) = connection(factory.clone()).await;
        request(
            &mut connection,
            3,
            "session/prompt",
            json!({"sessionId":id,
            "prompt":[{"type":"text","text":"EOF cutoff"}]}),
        );
        started(&mut connection, &factory).await;
        shutdown(&mut connection).await;
        assert!(connection.selection.current().is_none());
        assert!(connection.prompt.is_none());
    });
}

#[test]
fn session_model_response_follows_owned_save_and_survives_resume() {
    run(async {
        let factory = Arc::new(Factory::new());
        let (mut connection, id) = connection(factory.clone()).await;
        request(
            &mut connection,
            3,
            "session/set_config_option",
            json!({"sessionId":id,
            "configId":"model","value":"fixture/changed"}),
        );
        let (result, _) = response(&mut connection, 3).await;
        assert!(
            result.unwrap()["configOptions"]
                .as_array()
                .unwrap()
                .iter()
                .any(
                    |option| option["id"] == "model" && option["currentValue"] == "fixture/changed"
                )
        );
        request(
            &mut connection,
            4,
            "session/resume",
            json!({"sessionId":id,"cwd":factory.workspace}),
        );
        let (result, _) = response(&mut connection, 4).await;
        assert!(
            result.unwrap()["configOptions"]
                .as_array()
                .unwrap()
                .iter()
                .any(
                    |option| option["id"] == "model" && option["currentValue"] == "fixture/changed"
                )
        );
        assert!(!factory.provider_started.load(Ordering::Acquire));
        shutdown(&mut connection).await;
    });
}
