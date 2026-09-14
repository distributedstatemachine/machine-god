use super::*;
use crate::acp::{
    client_requests::NativeAcpClientRequests,
    driver::NativeAcpConnection,
    protocol::{AcpId, AcpMessage, decode_frame},
};
use crate::{NativeOwnedWorkerCompletion, NativeOwnedWorkerScope};
use serde_json::{Value, json};
use std::sync::{Mutex, Weak, mpsc};

struct ObservedHost {
    workers: NativeOwnedWorkerScope,
    completion: NativeOwnedWorkerCompletion,
    host: Weak<NativeReferenceHost>,
}

struct ObservedFactory {
    inner: Arc<Factory>,
    observed: Arc<Mutex<Option<ObservedHost>>>,
}

impl NativeAcpHostFactory for ObservedFactory {
    fn prepare(
        &self,
        workspace: PathBuf,
        network: NativeMcpNetworkRequirement,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeAcpPreparedHost, AcpSessionError>> {
        let prepared = self.inner.prepare(workspace, network, cancellation);
        let observed = self.observed.clone();
        Box::pin(async move {
            let prepared = prepared.await?;
            *observed.lock().unwrap() = Some(ObservedHost {
                workers: prepared.host.control_workers().unwrap(),
                completion: prepared.host.terminal_shutdown_completion().unwrap(),
                host: Arc::downgrade(&prepared.host),
            });
            Ok(prepared)
        })
    }

    fn list(
        &self,
        workspace: Option<PathBuf>,
        cursor: Option<NativeSessionCatalogCursor>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeSessionCatalogPage, NativeSessionCatalogReadError>> {
        self.inner.list(workspace, cursor, cancellation)
    }
}

fn request(connection: &mut NativeAcpConnection, id: i64, method: &str, params: Value) {
    connection
        .receive(
            AcpMessage::Request {
                id: AcpId::Integer(id),
                method: method.into(),
                params: Some(params),
            },
            200,
        )
        .unwrap();
}

async fn next(connection: &mut NativeAcpConnection) -> AcpMessage {
    let bytes = futures_util::future::poll_fn(|cx| connection.poll_output(cx, 200))
        .await
        .expect("connection still owns a response");
    decode_frame(&bytes).unwrap()
}

async fn response(connection: &mut NativeAcpConnection, expected: i64) -> Value {
    loop {
        if let AcpMessage::Response { id, outcome } = next(connection).await {
            assert_eq!(id, Some(AcpId::Integer(expected)));
            return outcome.unwrap();
        }
    }
}

async fn open_connection() -> (NativeAcpConnection, Value, ObservedHost) {
    let factory = Arc::new(Factory::new());
    let observed = Arc::new(Mutex::new(None));
    let mut connection = NativeAcpConnection::new(
        Arc::new(ObservedFactory {
            inner: factory.clone(),
            observed: observed.clone(),
        }),
        NativeAcpClientRequests::new().unwrap(),
    );
    request(
        &mut connection,
        1,
        "initialize",
        json!({"protocolVersion":1}),
    );
    response(&mut connection, 1).await;
    request(
        &mut connection,
        2,
        "session/new",
        json!({"cwd":factory.workspace}),
    );
    let selected = response(&mut connection, 2).await;
    // Drain the selected session's advertisement before testing that retirement
    // cannot expose another frame before actual settlement.
    let AcpMessage::Notification {
        params: Some(params),
        ..
    } = next(&mut connection).await
    else {
        panic!("selected session advertises commands");
    };
    assert_eq!(
        params["update"]["sessionUpdate"],
        "available_commands_update"
    );
    let observed = observed.lock().unwrap().take().unwrap();
    (connection, selected, observed)
}

#[test]
fn close_and_eof_wait_for_retired_host_when_unscoped_observer_admission_is_rejected() {
    for eof in [false, true] {
        run(async {
            let (mut connection, selected, observed) = open_connection().await;
            let (release, held) = mpsc::channel::<()>();
            let (started, ready) = tokio::sync::oneshot::channel();
            observed
                .workers
                .spawn(move || {
                    let _ = started.send(());
                    let _ = held.recv();
                })
                .unwrap();
            ready.await.unwrap();
            assert!(!observed.completion.is_complete());
            assert!(
                crate::owned_worker::with_rejected_unscoped_workers(|| {
                    crate::NativeOwnedWorkerSpawner::new().spawn(|| {})
                })
                .is_err()
            );

            if eof {
                connection.begin_shutdown();
            } else {
                request(
                    &mut connection,
                    3,
                    "session/close",
                    json!({"sessionId":selected["sessionId"]}),
                );
            }
            // Reject only unscoped admissions made on this polling thread.
            // Other tests and the real host worker keep their normal capacity.
            futures_util::future::poll_fn(|cx| {
                crate::owned_worker::with_rejected_unscoped_workers(|| {
                    let _ = connection.poll_progress(cx, 200);
                    assert!(
                        !connection.is_closed(),
                        "EOF cannot abandon the retired worker"
                    );
                    if observed.host.upgrade().is_none() {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                })
            })
            .await;
            assert!(!observed.completion.is_complete());
            futures_util::future::poll_fn(|cx| {
                crate::owned_worker::with_rejected_unscoped_workers(|| {
                    assert!(connection.poll_output(cx, 200).is_pending());
                    assert!(!connection.is_closed());
                });
                Poll::Ready(())
            })
            .await;

            // Dropping the sender also releases the worker if an assertion
            // unwinds; no test failure can leave a blocked collector job behind.
            drop(release);
            if eof {
                futures_util::future::poll_fn(|cx| {
                    crate::owned_worker::with_rejected_unscoped_workers(|| {
                        let _ = connection.poll_progress(cx, 200);
                        if connection.is_closed() {
                            Poll::Ready(())
                        } else {
                            Poll::Pending
                        }
                    })
                })
                .await;
            } else {
                let bytes = futures_util::future::poll_fn(|cx| {
                    crate::owned_worker::with_rejected_unscoped_workers(|| {
                        connection.poll_output(cx, 200)
                    })
                })
                .await
                .expect("settled close response");
                let AcpMessage::Response { id, outcome } = decode_frame(&bytes).unwrap() else {
                    panic!("close responds only after worker settlement");
                };
                assert_eq!(id, Some(AcpId::Integer(3)));
                assert_eq!(outcome.unwrap(), json!({}));
                connection.begin_shutdown();
                assert!(connection.is_closed());
            }
            assert!(observed.completion.is_complete());
            assert!(connection.error().is_none());
        });
    }
}
