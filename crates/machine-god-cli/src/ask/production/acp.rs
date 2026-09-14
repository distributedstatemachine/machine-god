//! ACP transport owns byte custody only; native owns the connection/session.

use super::{
    AskCommandOutcome, AskSignalControlSender, AskSignalController, AskSignals,
    acp_startup::{AcpHostFactory, CapturedAcpLaunch},
    output::{OutputAcknowledgement, OutputBridge, OutputWork, serve_output},
};
use machine_god_core::CancellationToken;
use machine_god_native::{
    NativeAcpConnection, NativeInteractiveInput, NativeInteractiveInputSource,
    NativeOwnedWorkerScope, TokioWebSearchDeadline,
    acp::{
        client_requests::NativeAcpClientRequests,
        protocol::{AcpFrameDecoder, AcpMessage, AcpProtocolError},
    },
};
use std::{
    future::poll_fn,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

const FINAL_OUTPUT_GRACE: Duration = Duration::from_secs(3);

#[cfg(test)]
mod composition_tests;
#[cfg(test)]
mod tests;

pub(super) fn execute(
    output: &mut dyn std::io::Write,
    controller: AskSignalController,
) -> (AskCommandOutcome, AskSignalController) {
    execute_with_capture(
        output,
        controller,
        super::piped_prompt::capture,
        CapturedAcpLaunch::capture,
    )
}

// The production entry and composed I/O fixture use identical acquisition,
// stdio acknowledgement, native session and cleanup code. Only explicit launch
// and input authorities differ; neither callback may supply a prepared host.
fn execute_with_capture(
    output: &mut dyn std::io::Write,
    mut controller: AskSignalController,
    capture_input: impl FnOnce() -> Result<NativeInteractiveInputSource, ()> + Send,
    capture_launch: impl FnOnce() -> Result<CapturedAcpLaunch, ()> + Send,
) -> (AskCommandOutcome, AskSignalController) {
    let control = controller.control();
    let Ok(signals) = controller.take_signals() else {
        return (AskCommandOutcome::OperationalFailure, controller);
    };
    let result = std::thread::scope(|scope| {
        let (work, received) = tokio::sync::mpsc::channel(1);
        let (acknowledged, acknowledgements) = tokio::sync::mpsc::channel(1);
        let worker = std::thread::Builder::new()
            .name("machine-god-acp".into())
            .spawn_scoped(scope, move || {
                run(
                    OutputBridge {
                        work,
                        acknowledgements,
                        tape: None,
                    },
                    signals,
                    &control,
                    capture_input,
                    capture_launch,
                )
            })
            .map_err(|_| ())?;
        serve_output(received, &acknowledged, output);
        worker.join().map_err(|_| ())?
    });
    let outcome = result.unwrap_or(AskCommandOutcome::OperationalFailure);
    let _ = controller.enter_final();
    (outcome, controller)
}

fn run(
    mut output: OutputBridge,
    mut signals: AskSignals,
    control: &AskSignalControlSender,
    capture_input: impl FnOnce() -> Result<NativeInteractiveInputSource, ()>,
    capture_launch: impl FnOnce() -> Result<CapturedAcpLaunch, ()>,
) -> Result<AskCommandOutcome, ()> {
    let source = capture_input()?;
    let (runtime, deadline) = TokioWebSearchDeadline::build_runtime_pair().map_err(|_| ())?;
    let clients = NativeAcpClientRequests::new().map_err(|_| ())?;
    let workers = NativeOwnedWorkerScope::new();
    let completion = workers.completion();
    let factory = AcpHostFactory::new(
        capture_launch()?,
        runtime.handle().clone(),
        Arc::new(deadline),
        clients.bridge(),
        clients.presenter(),
        workers.clone(),
    );
    control.activate_turn()?;
    let mut input = NativeInteractiveInput::new(source, CancellationToken::new());
    let input_completion = input.completion();
    let mut connection = NativeAcpConnection::new(Arc::new(factory), clients);
    let mut state = Transport::default();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runtime.block_on(poll_fn(|cx| {
            state.poll(cx, &mut connection, &mut input, &mut output, &mut signals)
        }));
    }));
    if let Err(payload) = result {
        std::mem::forget(payload);
        state.failed = true;
        connection.output_failed();
        runtime.block_on(poll_fn(|cx| {
            let _ = connection.poll_progress(cx, super::wall_clock_ms().unwrap_or(0));
            if connection.is_closed() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }));
    }
    drop(input);
    workers.close();
    // Connection closure has already settled all asynchronous host operations;
    // these exact collector joins cannot abandon a preparation or input child.
    let joined = input_completion.wait_on_worker().is_ok() & completion.wait_on_worker().is_ok();
    state.failed |= !joined;
    let settled = runtime.block_on(finish_output(
        &mut state,
        &mut connection,
        &mut output,
        &mut signals,
    ));
    drop(connection);
    let outcome = signals.first_observed.map_or_else(
        || {
            if !settled || state.output_failed {
                AskCommandOutcome::OutputFailure
            } else if state.failed {
                AskCommandOutcome::OperationalFailure
            } else {
                AskCommandOutcome::Completed
            }
        },
        super::AskSignal::outcome,
    );
    if !settled {
        // Stdout's caller may be blocked in write/flush. The existing guardian
        // exits only here, after the exact native and input owners were joined.
        control.finish(outcome.exit_code())?;
    }
    Ok(outcome)
}

#[derive(Default)]
struct Transport {
    decoder: AcpFrameDecoder,
    chunk: Vec<u8>,
    offset: usize,
    pending: Option<Result<AcpMessage, AcpProtocolError>>,
    writing: Option<WritePhase>,
    stopping: bool,
    failed: bool,
    output_failed: bool,
}
#[derive(Clone, Copy)]
enum WritePhase {
    Write,
    Flush,
}

impl Transport {
    fn poll(
        &mut self,
        cx: &mut Context<'_>,
        connection: &mut NativeAcpConnection,
        input: &mut NativeInteractiveInput,
        output: &mut OutputBridge,
        signals: &mut AskSignals,
    ) -> Poll<()> {
        if signals.poll_signal(cx).is_ready() {
            self.stopping = true;
        }
        if self.poll_written(cx, output).is_err() {
            self.output_failed = true;
            self.stopping = true;
            connection.output_failed();
        }
        if self.stopping {
            self.pending = None;
            self.chunk.clear();
            self.offset = 0;
            let _ = self.decoder.finish();
            input.request_stop();
            connection.begin_shutdown();
        }
        let now = super::wall_clock_ms().unwrap_or_else(|()| {
            self.failed = true;
            self.stopping = true;
            connection.begin_shutdown();
            0
        });
        let _ = connection.poll_progress(cx, now);
        if self.writing.is_none()
            && let Poll::Ready(Some(frame)) = connection.poll_output(cx, now)
        {
            if output.work.try_send(OutputWork::Write(frame)).is_err() {
                self.output_failed = true;
                self.stopping = true;
                connection.output_failed();
            } else {
                self.writing = Some(WritePhase::Write);
            }
        }
        if connection.is_closed() {
            self.failed |= connection.error().is_some();
            return Poll::Ready(());
        }
        if !self.stopping {
            self.poll_input(cx, connection, input, now);
        }
        Poll::Pending
    }

    fn poll_input(
        &mut self,
        cx: &mut Context<'_>,
        connection: &mut NativeAcpConnection,
        input: &mut NativeInteractiveInput,
        now: i64,
    ) {
        if self.pending.is_none() && self.offset < self.chunk.len() {
            let mut bytes = &self.chunk[self.offset..];
            self.pending = self.decoder.next(&mut bytes);
            self.offset = self.chunk.len() - bytes.len();
        }
        if let Some(pending) = self.pending.take() {
            self.pending = match pending {
                Ok(message) => connection.receive(message, now).err().map(Ok),
                Err(error) => connection.receive_error(error).err().map(Err),
            };
            if self.pending.is_none() {
                cx.waker().wake_by_ref();
            } else {
                // A complete backpressured frame must not hide a disconnected
                // pipe writer. Observe HUP without consuming another byte or
                // changing the normal demand-gated input/EOF contract.
                match input.poll_pipe_peer_closed(cx) {
                    Poll::Ready(Ok(true)) => self.stopping = true,
                    Poll::Ready(Err(_)) => {
                        self.failed = true;
                        self.stopping = true;
                    }
                    Poll::Pending | Poll::Ready(Ok(false)) => {}
                }
                if self.stopping {
                    cx.waker().wake_by_ref();
                }
            }
            return;
        }
        if self.offset < self.chunk.len() {
            cx.waker().wake_by_ref();
            return;
        }
        match input.poll_chunk(cx) {
            Poll::Pending => {}
            Poll::Ready(Ok(Some(chunk))) => {
                self.chunk.clear();
                self.chunk.extend_from_slice(chunk.as_bytes());
                self.offset = 0;
                cx.waker().wake_by_ref();
            }
            Poll::Ready(Ok(None)) => {
                if let Some(error) = self.decoder.finish() {
                    let _ = connection.receive_error(error);
                }
                // EOF is a terminal cutoff, including when a truncation error
                // cannot enter a blocked native reply lane. Never delay actual
                // cancellation waiting for a peer which has already left.
                self.stopping = true;
                cx.waker().wake_by_ref();
            }
            Poll::Ready(Err(_)) => {
                self.failed = true;
                self.stopping = true;
                cx.waker().wake_by_ref();
            }
        }
    }

    fn poll_written(&mut self, cx: &mut Context<'_>, output: &mut OutputBridge) -> Result<(), ()> {
        let Some(phase) = self.writing else {
            return Ok(());
        };
        match output.poll_acknowledgement(cx) {
            Poll::Pending => Ok(()),
            Poll::Ready(Some(OutputAcknowledgement::Succeeded)) => {
                self.writing = None;
                self.writing = match phase {
                    WritePhase::Write => {
                        output.work.try_send(OutputWork::Flush).map_err(|_| ())?;
                        Some(WritePhase::Flush)
                    }
                    WritePhase::Flush => None,
                };
                cx.waker().wake_by_ref();
                Ok(())
            }
            Poll::Ready(_) => {
                self.writing = None;
                Err(())
            }
        }
    }
}

async fn finish_output(
    state: &mut Transport,
    connection: &mut NativeAcpConnection,
    output: &mut OutputBridge,
    signals: &mut AskSignals,
) -> bool {
    let mut deadline = tokio::time::Instant::now() + FINAL_OUTPUT_GRACE;
    if signals.first_observed.is_some() {
        deadline = tokio::time::Instant::now() + super::SIGNAL_OUTPUT_GRACE;
    }
    let timer = tokio::time::sleep_until(deadline);
    tokio::pin!(timer);
    poll_fn(|cx| {
        use std::future::Future;
        if signals.poll_signal(cx).is_ready() {
            let signal_deadline = tokio::time::Instant::now() + super::SIGNAL_OUTPUT_GRACE;
            if signal_deadline < deadline {
                deadline = signal_deadline;
                timer.as_mut().reset(deadline);
            }
        }
        if state.poll_written(cx, output).is_err() {
            return Poll::Ready(false);
        }
        if state.writing.is_none() {
            match connection.poll_output(cx, super::wall_clock_ms().unwrap_or(0)) {
                Poll::Ready(Some(frame)) => {
                    if output.work.try_send(OutputWork::Write(frame)).is_err() {
                        return Poll::Ready(false);
                    }
                    state.writing = Some(WritePhase::Write);
                    cx.waker().wake_by_ref();
                }
                Poll::Ready(None) => return Poll::Ready(true),
                Poll::Pending => {}
            }
        }
        timer.as_mut().poll(cx).map(|()| false)
    })
    .await
}
