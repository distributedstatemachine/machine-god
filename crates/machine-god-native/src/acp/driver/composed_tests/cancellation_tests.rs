use super::*;

fn prompt(connection: &mut NativeAcpConnection, request_id: i64, session_id: &str) {
    request(
        connection,
        request_id,
        "session/prompt",
        json!({"sessionId":session_id,"prompt":[{"type":"text","text":"cancel preparation"}]}),
    );
}

#[test]
fn cancellation_before_first_poll_returns_cancelled_and_keeps_session_usable() {
    run(async {
        let factory = Arc::new(Factory::new());
        let (mut connection, id) = connection(factory.clone()).await;
        let runtime = connection.selection.current().unwrap().runtime().clone();
        let before = runtime.record_snapshot();
        prompt(&mut connection, 3, &id);
        cancel(&mut connection, &id);
        assert_eq!(
            response(&mut connection, 3).await.0.unwrap()["stopReason"],
            "cancelled"
        );
        assert!(!factory.provider_started.load(Ordering::Acquire));
        assert_eq!(runtime.record_snapshot(), before);
        assert!(!runtime.status().active);
        prompt(&mut connection, 4, &id);
        started(&mut connection, &factory).await;
        cancel(&mut connection, &id);
        assert_eq!(
            response(&mut connection, 4).await.0.unwrap()["stopReason"],
            "cancelled"
        );
        shutdown(&mut connection).await;
    });
}

async fn held_preparation(connection: &mut NativeAcpConnection) -> std::sync::mpsc::SyncSender<()> {
    let (entered, release) = connection
        .selection
        .current()
        .unwrap()
        .runtime()
        .block_next_acp_resource_read_for_test();
    futures_util::future::poll_fn(|cx| {
        let _ = connection.poll_progress(cx, 200);
        match entered.try_recv() {
            Ok(()) => Poll::Ready(()),
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(error) => panic!("owned reader exited before entering: {error}"),
        }
    })
    .await;
    release
}

#[test]
fn cancellation_reply_waits_for_actual_resource_worker_settlement() {
    run(async {
        let factory = Arc::new(Factory::new());
        let (mut connection, id) = connection(factory.clone()).await;
        let runtime = connection.selection.current().unwrap().runtime().clone();
        let before = runtime.record_snapshot();
        prompt(&mut connection, 3, &id);
        let release = held_preparation(&mut connection).await;
        cancel(&mut connection, &id);
        let waker = futures_util::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        for _ in 0..128 {
            match connection.poll_output(&mut cx, 200) {
                Poll::Ready(Some(frame)) => assert!(matches!(
                    decode_frame(&frame).unwrap(),
                    AcpMessage::Notification { .. }
                )),
                Poll::Pending => break,
                Poll::Ready(None) => panic!("connection closed before worker settlement"),
            }
        }
        assert!(runtime.status().active);
        assert!(connection.prompt.is_some());
        assert!(!factory.provider_started.load(Ordering::Acquire));
        release.send(()).unwrap();
        assert_eq!(
            response(&mut connection, 3).await.0.unwrap()["stopReason"],
            "cancelled"
        );
        assert!(!runtime.status().active);
        assert_eq!(runtime.record_snapshot(), before);
        assert!(!factory.provider_started.load(Ordering::Acquire));
        shutdown(&mut connection).await;
    });
}

#[test]
fn eof_during_preparation_waits_for_worker_without_native_failure() {
    run(async {
        let factory = Arc::new(Factory::new());
        let (mut connection, id) = connection(factory.clone()).await;
        prompt(&mut connection, 3, &id);
        let release = held_preparation(&mut connection).await;
        connection.begin_shutdown();
        let waker = futures_util::task::noop_waker();
        let _ = connection.poll_progress(&mut Context::from_waker(&waker), 200);
        assert!(!connection.is_closed());
        assert!(connection.error().is_none());
        assert!(!factory.provider_started.load(Ordering::Acquire));
        release.send(()).unwrap();
        shutdown(&mut connection).await;
        assert!(!factory.provider_started.load(Ordering::Acquire));
        assert!(connection.selection.current().is_none());
    });
}
