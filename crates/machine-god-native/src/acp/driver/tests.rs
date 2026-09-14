use super::*;
use crate::NativeSessionCatalogCursor;
use crate::acp::{
    protocol::decode_frame, selection::NativeAcpPreparedHost, session::AcpSessionError,
};
use futures_util::task::noop_waker;
use serde_json::json;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct Factory {
    preparations: Arc<AtomicUsize>,
    lists: Arc<AtomicUsize>,
}
impl NativeAcpHostFactory for Factory {
    fn prepare(
        &self,
        _: PathBuf,
        _: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeAcpPreparedHost, AcpSessionError>> {
        let calls = self.preparations.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(AcpSessionError::Unavailable)
        })
    }
    fn list(
        &self,
        _: Option<PathBuf>,
        _: Option<NativeSessionCatalogCursor>,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeSessionCatalogPage, NativeSessionCatalogReadError>> {
        let calls = self.lists.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            cancel.cancelled().await;
            Err(NativeSessionCatalogReadError::Cancelled)
        })
    }
}
fn message(id: i64, method: &str, params: Value) -> AcpMessage {
    AcpMessage::Request {
        id: AcpId::Integer(id),
        method: method.into(),
        params: Some(params),
    }
}
fn poll(connection: &mut NativeAcpConnection) -> Poll<Option<Vec<u8>>> {
    connection.poll_output(&mut Context::from_waker(&noop_waker()), 1)
}
fn reply(connection: &mut NativeAcpConnection) -> AcpMessage {
    let Poll::Ready(Some(frame)) = poll(connection) else {
        panic!("expected one reply")
    };
    decode_frame(&frame).unwrap()
}
fn initialized(factory: Arc<Factory>) -> NativeAcpConnection {
    let mut connection = NativeAcpConnection::new(factory, NativeAcpClientRequests::new().unwrap());
    connection
        .receive(message(1, "initialize", json!({"protocolVersion":1})), 1)
        .unwrap();
    assert!(matches!(
        reply(&mut connection),
        AcpMessage::Response { outcome: Ok(_), .. }
    ));
    connection
}

#[test]
fn initialize_is_inert_and_normal_backpressure_returns_exact_message() {
    let factory = Arc::new(Factory::default());
    let mut connection =
        NativeAcpConnection::new(factory.clone(), NativeAcpClientRequests::new().unwrap());
    connection
        .receive(message(1, "initialize", json!({"protocolVersion":1})), 1)
        .unwrap();
    let pending = message(2, "session/new", json!({"cwd":"/test"}));
    assert_eq!(connection.receive(pending.clone(), 1).unwrap_err(), pending);
    assert_eq!(factory.preparations.load(Ordering::SeqCst), 0);
    let _ = reply(&mut connection);
    assert_eq!(factory.preparations.load(Ordering::SeqCst), 0);
    connection.begin_shutdown();
    assert!(matches!(poll(&mut connection), Poll::Ready(None)));
}

#[test]
fn malformed_frames_are_bounded_and_eof_preserves_one_ready_reply() {
    let mut connection = initialized(Arc::new(Factory::default()));
    connection
        .receive_error(AcpProtocolError::ParseError)
        .unwrap();
    assert_eq!(
        connection.receive_error(AcpProtocolError::FrameTooLarge),
        Err(AcpProtocolError::FrameTooLarge)
    );
    connection.begin_shutdown();
    let _ = connection.poll_progress(&mut Context::from_waker(&noop_waker()), 1);
    assert!(connection.is_closed());
    assert!(connection.has_output());
    assert!(matches!(
        reply(&mut connection),
        AcpMessage::Response {
            id: None,
            outcome: Err(AcpRpcError { code: -32700, .. })
        }
    ));
    assert!(matches!(poll(&mut connection), Poll::Ready(None)));
}

#[test]
fn list_cancellation_is_driven_while_no_output_slot_is_available() {
    let factory = Arc::new(Factory::default());
    let mut connection = initialized(factory.clone());
    connection
        .receive(message(2, "session/list", json!({})), 1)
        .unwrap();
    assert_eq!(factory.lists.load(Ordering::SeqCst), 0);
    let _ = connection.poll_progress(&mut Context::from_waker(&noop_waker()), 1);
    assert_eq!(factory.lists.load(Ordering::SeqCst), 1);
    assert!(!connection.is_closed());
    connection.begin_shutdown();
    let _ = connection.poll_progress(&mut Context::from_waker(&noop_waker()), 1);
    assert!(connection.is_closed());
    assert_eq!(factory.lists.load(Ordering::SeqCst), 1);
}

#[test]
fn initialization_is_required_and_unknown_methods_never_prepare_hosts() {
    let factory = Arc::new(Factory::default());
    let mut connection =
        NativeAcpConnection::new(factory.clone(), NativeAcpClientRequests::new().unwrap());
    connection
        .receive(message(1, "session/new", json!({"cwd":"/test"})), 1)
        .unwrap();
    assert!(matches!(
        reply(&mut connection),
        AcpMessage::Response {
            outcome: Err(AcpRpcError { code: -32600, .. }),
            ..
        }
    ));
    connection
        .receive(message(2, "legacy/session/remove", json!({})), 1)
        .unwrap();
    assert!(matches!(
        reply(&mut connection),
        AcpMessage::Response {
            outcome: Err(AcpRpcError { code: -32601, .. }),
            ..
        }
    ));
    assert_eq!(factory.preparations.load(Ordering::SeqCst), 0);
    connection.begin_shutdown();
    assert!(matches!(poll(&mut connection), Poll::Ready(None)));
}

#[test]
fn failed_preparation_is_correlated_and_does_not_initialize_an_actor() {
    let factory = Arc::new(Factory::default());
    let mut connection = initialized(factory.clone());
    connection
        .receive(message(7, "session/new", json!({"cwd":"/test"})), 1)
        .unwrap();
    assert_eq!(factory.preparations.load(Ordering::SeqCst), 0);
    assert!(connection.selection.current().is_none());
    let result = reply(&mut connection);
    assert!(matches!(
        result,
        AcpMessage::Response {
            id: Some(AcpId::Integer(7)),
            outcome: Err(_)
        }
    ));
    assert_eq!(factory.preparations.load(Ordering::SeqCst), 1);
    assert!(connection.selection.current().is_none());
    assert!(connection.can_receive());
    connection.begin_shutdown();
    assert!(matches!(poll(&mut connection), Poll::Ready(None)));
}

#[test]
fn constructed_envelope_ids_are_bounded_before_reply_retention() {
    let mut connection = initialized(Arc::new(Factory::default()));
    connection
        .receive(
            AcpMessage::Request {
                id: AcpId::String("x".repeat(protocol::ACP_MAX_ID_BYTES + 1)),
                method: "session/list".to_owned(),
                params: Some(json!({})),
            },
            1,
        )
        .unwrap();
    assert!(matches!(
        reply(&mut connection),
        AcpMessage::Response {
            id: None,
            outcome: Err(AcpRpcError { code: -32600, .. })
        }
    ));
    connection.begin_shutdown();
    assert!(matches!(poll(&mut connection), Poll::Ready(None)));
}

#[test]
fn discarded_constructed_notifications_and_replies_have_iterative_cleanup() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            fn deep() -> Value {
                let mut value = Value::Null;
                for _ in 0..4096 {
                    value = Value::Array(vec![value]);
                }
                value
            }
            let mut connection = initialized(Arc::new(Factory::default()));
            let mut notification = serde_json::Map::new();
            notification.insert("ignored".to_owned(), deep());
            connection
                .receive(
                    AcpMessage::Notification {
                        method: "ignored".to_owned(),
                        params: Some(Value::Object(notification)),
                    },
                    1,
                )
                .unwrap();
            connection
                .receive(
                    AcpMessage::Response {
                        id: Some(AcpId::Integer(9)),
                        outcome: Ok(deep()),
                    },
                    1,
                )
                .unwrap();
            connection
                .receive(
                    AcpMessage::Response {
                        id: Some(AcpId::Integer(9)),
                        outcome: Err(AcpRpcError {
                            code: -1,
                            message: "discarded remote text".to_owned(),
                            data: Some(deep()),
                        }),
                    },
                    1,
                )
                .unwrap();
            assert!(connection.reply.is_none());
            connection.begin_shutdown();
            assert!(matches!(poll(&mut connection), Poll::Ready(None)));
        })
        .unwrap()
        .join()
        .unwrap();
}
