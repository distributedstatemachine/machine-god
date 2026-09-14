use super::*;
use machine_god_core::BoxFuture;
use machine_god_native::{
    NativeInteractiveInputSource, NativeSessionCatalogCursor, NativeSessionCatalogPage,
    NativeSessionCatalogReadError,
    acp::{
        selection::{NativeAcpHostFactory, NativeAcpPreparedHost},
        session::AcpSessionError,
    },
};
use std::path::PathBuf;
use std::task::Waker;

struct NoHosts;
impl NativeAcpHostFactory for NoHosts {
    fn prepare(
        &self,
        _: PathBuf,
        _: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeAcpPreparedHost, AcpSessionError>> {
        panic!("initialize and EOF do not acquire a host");
    }
    fn list(
        &self,
        _: Option<PathBuf>,
        _: Option<NativeSessionCatalogCursor>,
        _: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeSessionCatalogPage, NativeSessionCatalogReadError>> {
        panic!("initialize and EOF do not list sessions");
    }
}

struct PendingList;
impl NativeAcpHostFactory for PendingList {
    fn prepare(
        &self,
        workspace: PathBuf,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeAcpPreparedHost, AcpSessionError>> {
        NoHosts.prepare(workspace, cancellation)
    }
    fn list(
        &self,
        _: Option<PathBuf>,
        _: Option<NativeSessionCatalogCursor>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeSessionCatalogPage, NativeSessionCatalogReadError>> {
        Box::pin(async move {
            cancellation.cancelled().await;
            Err(NativeSessionCatalogReadError::Cancelled)
        })
    }
}

#[test]
fn complete_backpressured_frame_observes_real_pipe_disconnect_before_output_grace() {
    use std::io::Write as _;
    use std::os::fd::OwnedFd;
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let mut connection = NativeAcpConnection::new(Arc::new(PendingList), NativeAcpClientRequests::new().unwrap());
        connection.receive(machine_god_native::acp::protocol::decode_frame(
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}"#).unwrap(), 0).unwrap();
        assert!(poll_fn(|cx| connection.poll_output(cx, 0)).await.is_some());
        connection.receive(machine_god_native::acp::protocol::decode_frame(
            br#"{"jsonrpc":"2.0","id":2,"method":"session/list"}"#).unwrap(), 0).unwrap();
        let (read, mut write) = std::io::pipe().unwrap();
        let original = rustix::fs::fcntl_getfl(&read).unwrap() | rustix::fs::OFlags::NONBLOCK;
        rustix::fs::fcntl_setfl(&read, original).unwrap();
        let alias = read.try_clone().unwrap();
        let mut input = NativeInteractiveInput::new(NativeInteractiveInputSource::PreserveNonblocking(OwnedFd::from(read).into()), CancellationToken::new());
        let completion = input.completion();
        let (work, _received) = tokio::sync::mpsc::channel(1);
        let (_acknowledged, acknowledgements) = tokio::sync::mpsc::channel(1);
        let mut output = OutputBridge { work, acknowledgements, tape: None };
        let (_signal_sender, receiver) = tokio::sync::mpsc::channel(1);
        let mut signals = AskSignals::new(receiver);
        let mut state = Transport { writing: Some(WritePhase::Write), ..Transport::default() };
        write.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"session/list\"}\n").unwrap();
        tokio::time::timeout(Duration::from_secs(2), poll_fn(|cx| {
            assert!(state.poll(cx, &mut connection, &mut input, &mut output, &mut signals).is_pending());
            if state.pending.is_some() { Poll::Ready(()) } else { Poll::Pending }
        })).await.unwrap();
        assert!(!connection.is_closed());
        assert!(!state.stopping);
        drop(write);
        tokio::time::timeout(Duration::from_secs(2), poll_fn(|cx| state.poll(cx, &mut connection, &mut input, &mut output, &mut signals))).await.unwrap();
        assert!(connection.is_closed(), "the admitted native list must actually settle");
        assert!(state.pending.is_none());
        assert!(state.writing.is_some(), "output remains blocked throughout native settlement");
        drop(input);
        completion.wait_on_worker().unwrap();
        assert!(completion.is_complete());
        assert_eq!(rustix::fs::fcntl_getfl(&alias).unwrap(), original);
        tokio::time::pause();
        let start = tokio::time::Instant::now();
        assert!(!finish_output(&mut state, &mut connection, &mut output, &mut signals).await);
        assert_eq!(tokio::time::Instant::now() - start, FINAL_OUTPUT_GRACE);
    });
}

#[test]
fn truncated_eof_cancels_pending_control_even_when_error_reply_cannot_be_admitted() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut connection = NativeAcpConnection::new(
                Arc::new(PendingList),
                NativeAcpClientRequests::new().unwrap(),
            );
            let initialize = machine_god_native::acp::protocol::decode_frame(
                br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}"#,
            )
            .unwrap();
            connection.receive(initialize, 0).unwrap();
            assert!(poll_fn(|cx| connection.poll_output(cx, 0)).await.is_some());
            let list = machine_god_native::acp::protocol::decode_frame(
                br#"{"jsonrpc":"2.0","id":2,"method":"session/list","params":{}}"#,
            )
            .unwrap();
            connection.receive(list, 0).unwrap();
            connection
                .receive_error(AcpProtocolError::ParseError)
                .unwrap();
            assert_eq!(
                connection.receive_error(AcpProtocolError::TruncatedFrame),
                Err(AcpProtocolError::TruncatedFrame)
            );
            let (work, _received) = tokio::sync::mpsc::channel(1);
            let (_acknowledged, acknowledgements) = tokio::sync::mpsc::channel(1);
            let mut output = OutputBridge {
                work,
                acknowledgements,
                tape: None,
            };
            let (_signal_sender, receiver) = tokio::sync::mpsc::channel(1);
            let mut signals = AskSignals::new(receiver);
            let mut input = NativeInteractiveInput::new(
                NativeInteractiveInputSource::Disabled,
                CancellationToken::new(),
            );
            let mut state = Transport {
                writing: Some(WritePhase::Write),
                ..Transport::default()
            };
            assert!(state.decoder.next(&mut b"{".as_slice()).is_none());
            poll_fn(|cx| state.poll(cx, &mut connection, &mut input, &mut output, &mut signals))
                .await;
            assert!(connection.is_closed());
            assert!(connection.has_output());
            assert!(state.pending.is_none());
        });
}

#[test]
fn eof_drives_native_shutdown_behind_blocked_stdout_then_uses_one_final_grace() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap()
        .block_on(async {
            for signal in [None, Some(super::super::AskSignal::Interrupt)] {
                let (work, _received) = tokio::sync::mpsc::channel(1);
                let (_acknowledged, acknowledgements) = tokio::sync::mpsc::channel(1);
                let mut output = OutputBridge {
                    work,
                    acknowledgements,
                    tape: None,
                };
                let (_signal_sender, receiver) = tokio::sync::mpsc::channel(1);
                let mut signals = AskSignals::new(receiver);
                let mut input = NativeInteractiveInput::new(
                    NativeInteractiveInputSource::Disabled,
                    CancellationToken::new(),
                );
                let completion = input.completion();
                let clients = NativeAcpClientRequests::new().unwrap();
                let mut connection = NativeAcpConnection::new(Arc::new(NoHosts), clients);
                let initialize = machine_god_native::acp::protocol::decode_frame(
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}"#,
        )
        .unwrap();
                connection.receive(initialize, 0).unwrap();
                let mut state = Transport {
                    writing: Some(WritePhase::Write),
                    ..Transport::default()
                };
                poll_fn(|cx| {
                    state.poll(cx, &mut connection, &mut input, &mut output, &mut signals)
                })
                .await;
                assert!(connection.is_closed());
                assert!(connection.has_output());
                drop(input);
                assert!(completion.is_complete());
                signals.first_observed = signal;
                let start = tokio::time::Instant::now();
                assert!(
                    !finish_output(&mut state, &mut connection, &mut output, &mut signals).await
                );
                let expected = if signal.is_some() {
                    super::super::SIGNAL_OUTPUT_GRACE
                } else {
                    FINAL_OUTPUT_GRACE
                };
                assert_eq!(tokio::time::Instant::now() - start, expected);
                assert!(connection.has_output());
            }
        });
}

#[test]
fn frame_slot_is_retained_through_write_and_flush_acknowledgements() {
    let (work, mut received) = tokio::sync::mpsc::channel(1);
    let (acknowledged, acknowledgements) = tokio::sync::mpsc::channel(1);
    let mut output = OutputBridge {
        work,
        acknowledgements,
        tape: None,
    };
    let mut state = Transport {
        writing: Some(WritePhase::Write),
        ..Transport::default()
    };
    let mut cx = Context::from_waker(Waker::noop());
    assert!(state.poll_written(&mut cx, &mut output).is_ok());
    assert!(matches!(state.writing, Some(WritePhase::Write)));
    assert!(received.try_recv().is_err());
    acknowledged
        .try_send(OutputAcknowledgement::Succeeded)
        .unwrap();
    assert!(state.poll_written(&mut cx, &mut output).is_ok());
    assert!(matches!(state.writing, Some(WritePhase::Flush)));
    assert!(matches!(received.try_recv(), Ok(OutputWork::Flush)));
    assert!(state.poll_written(&mut cx, &mut output).is_ok());
    assert!(state.writing.is_some());
    acknowledged
        .try_send(OutputAcknowledgement::Succeeded)
        .unwrap();
    assert!(state.poll_written(&mut cx, &mut output).is_ok());
    assert!(state.writing.is_none());
}

#[test]
fn failed_and_disconnected_output_release_frame_without_retrying_bytes() {
    for disconnect in [false, true] {
        let (work, mut received) = tokio::sync::mpsc::channel(1);
        let (acknowledged, acknowledgements) = tokio::sync::mpsc::channel(1);
        let mut output = OutputBridge {
            work,
            acknowledgements,
            tape: None,
        };
        let mut state = Transport {
            writing: Some(WritePhase::Write),
            ..Transport::default()
        };
        if !disconnect {
            acknowledged
                .try_send(OutputAcknowledgement::Failed)
                .unwrap();
        }
        drop(acknowledged);
        let mut cx = Context::from_waker(Waker::noop());
        assert!(state.poll_written(&mut cx, &mut output).is_err());
        assert!(state.writing.is_none());
        assert!(received.try_recv().is_err());
    }
}
