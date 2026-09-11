use super::super::protocol::{NdjsonDecoder, WireError};
use super::super::submission::{McpSubmissionWrite, McpSubmissionWriter};
use super::McpStdioReadEnd;
use super::{
    Arc, CancellationToken, Context, Future, Instant, MAX_MCP_STDIO_FRAMES, McpStdioControl,
    McpStdioError, McpStdioWriteReceipt, Payload, Pin, Poll, Response, Result, Shared, VecDeque,
    failed,
};
use crate::terminal_captured_exec::GatedProcess;
use std::io;
use std::os::unix::net::UnixStream;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(5);
const READ_BYTES: usize = 16 * 1024;

struct ReadState<'a> {
    decoder: NdjsonDecoder,
    shared: &'a Shared,
    bytes: [u8; READ_BYTES],
    start: usize,
    end: usize,
}
impl Drop for ReadState<'_> {
    fn drop(&mut self) {
        finalize_read(self.shared, &mut self.decoder, false);
    }
}
fn finalize_read(shared: &Shared, decoder: &mut NdjsonDecoder, eof: bool) -> bool {
    {
        let state = shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.read_end.is_some() {
            return state.buffered_partial_frame;
        }
    }
    let result = decoder.finish();
    let partial = result == Err(WireError::IncompleteFrame);
    let read_end = if !eof {
        McpStdioReadEnd::Unclassified
    } else if partial {
        McpStdioReadEnd::IncompleteEof
    } else if result.is_ok() {
        McpStdioReadEnd::CleanEof
    } else {
        McpStdioReadEnd::Unclassified
    };
    let mut state = shared
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.read_end = Some(read_end);
    state.buffered_partial_frame = partial;
    partial
}

struct SocketWriter {
    input: Arc<UnixStream>,
    connection: CancellationToken,
    host: CancellationToken,
    request: CancellationToken,
    deadline: Instant,
}
impl SocketWriter {
    fn live(&self) -> io::Result<()> {
        if self.connection.is_cancelled()
            || self.host.is_cancelled()
            || self.request.is_cancelled()
            || Instant::now() >= self.deadline
        {
            Err(io::Error::other("MCP stdio writer stopped"))
        } else {
            Ok(())
        }
    }
}
impl McpSubmissionWriter for SocketWriter {
    fn poll_write(&mut self, _: &mut Context<'_>, bytes: &[u8]) -> Poll<io::Result<usize>> {
        if let Err(error) = self.live() {
            return Poll::Ready(Err(error));
        }
        match rustix::io::write(&*self.input, &bytes[..bytes.len().min(READ_BYTES)]) {
            Err(rustix::io::Errno::AGAIN | rustix::io::Errno::INTR) => Poll::Pending,
            result => Poll::Ready(result.map_err(Into::into)),
        }
    }
    fn poll_flush(&mut self, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(self.live())
    }
}
enum Write {
    Tool(Box<McpSubmissionWrite<SocketWriter>>),
    Control {
        control: McpStdioControl,
        offset: usize,
        attempted: bool,
    },
}
struct Active {
    write: Write,
    deadline: Instant,
    cancel: CancellationToken,
    connection: CancellationToken,
    host: CancellationToken,
    response: Arc<Response<McpStdioWriteReceipt>>,
}
impl Active {
    fn receipt(&self, outcome: Result<()>) -> McpStdioWriteReceipt {
        let (attempted, acknowledged_bytes) = match &self.write {
            Write::Tool(write) => (write.was_attempted(), write.acknowledged_bytes()),
            Write::Control {
                offset, attempted, ..
            } => (*attempted, *offset),
        };
        McpStdioWriteReceipt {
            outcome,
            attempted,
            acknowledged_bytes,
        }
    }
    fn poll(&mut self, input: &Arc<UnixStream>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        if self.cancel.is_cancelled() || self.connection.is_cancelled() || self.host.is_cancelled()
        {
            return Poll::Ready(Err(McpStdioError::Cancelled));
        }
        if Instant::now() >= self.deadline {
            return Poll::Ready(Err(McpStdioError::Deadline));
        }
        match &mut self.write {
            Write::Tool(write) => Pin::new(&mut **write)
                .poll(cx)
                .map(|result| result.map_err(McpStdioError::Submission)),
            Write::Control {
                control,
                offset,
                attempted,
            } => {
                *attempted = true;
                if *offset == control.bytes.len() {
                    return Poll::Ready(Ok(()));
                }
                match (SocketWriter {
                    input: input.clone(),
                    connection: self.connection.clone(),
                    host: self.host.clone(),
                    request: self.cancel.clone(),
                    deadline: self.deadline,
                })
                .poll_write(cx, &control.bytes[*offset..])
                {
                    Poll::Ready(Ok(0) | Err(_)) => Poll::Ready(Err(McpStdioError::Process)),
                    Poll::Ready(Ok(count)) => {
                        *offset += count;
                        Poll::Pending
                    }
                    Poll::Pending => Poll::Pending,
                }
            }
        }
    }
}
impl Drop for Active {
    fn drop(&mut self) {
        // Also resolves an in-flight waiter if a panic unwinds the owned worker.
        self.response
            .complete_if_empty(Ok(self.receipt(Err(McpStdioError::Process))));
    }
}

pub(super) fn run(mut process: GatedProcess, shared: &Shared, cancellation: &CancellationToken) {
    let input = Arc::new(process.input);
    let mut active = None;
    let error = run_io(&input, &process.output, shared, cancellation, &mut active);
    if let Some(active) = active.take() {
        active.response.complete(Ok(active.receipt(Err(error))));
    }
    shared.finish(error);
    // EOF gets one bounded grace period; all cleanup stays on this worker.
    let _ = input.shutdown(std::net::Shutdown::Write);
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        match process.process.process.terminal_poll() {
            Ok(None) => std::thread::sleep(POLL_INTERVAL),
            _ => break,
        }
    }
    drop(input);
    drop(process.output);
    let _ = process.process.close(true);
}

fn run_io(
    input: &Arc<UnixStream>,
    output: &std::io::PipeReader,
    shared: &Shared,
    cancellation: &CancellationToken,
    active: &mut Option<Active>,
) -> McpStdioError {
    let Ok(decoder) = NdjsonDecoder::new(shared.limits) else {
        return McpStdioError::Invalid;
    };
    let mut reader = ReadState {
        decoder,
        shared,
        bytes: [0; READ_BYTES],
        start: 0,
        end: 0,
    };
    let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
    loop {
        let mut progressed = false;
        if cancellation.is_cancelled() || shared.stop.is_cancelled() {
            return McpStdioError::Cancelled;
        }
        if let Err(error) = prune(shared, &mut cx) {
            return error;
        }
        if active.is_none() {
            let queued = match shared.state.lock() {
                Ok(mut state) => state.queue.pop_front(),
                Err(_) => return McpStdioError::Closed,
            };
            if let Some(queued) = queued {
                if let Payload::Tool(submission) = &queued.payload
                    && let Err(error) = shared.affinity(submission)
                {
                    settle(shared, &queued.response, failed(error));
                    continue;
                }
                *active = Some(Active {
                    write: match queued.payload {
                        Payload::Tool(submission) => {
                            Write::Tool(Box::new(submission.into_writer(SocketWriter {
                                input: input.clone(),
                                connection: shared.stop.clone(),
                                host: cancellation.clone(),
                                request: queued.cancel.clone(),
                                deadline: queued.deadline,
                            })))
                        }
                        Payload::Control(control) => Write::Control {
                            control,
                            offset: 0,
                            attempted: false,
                        },
                    },
                    deadline: queued.deadline,
                    cancel: queued.cancel,
                    connection: shared.stop.clone(),
                    host: cancellation.clone(),
                    response: queued.response,
                });
            }
        }
        if let Some(writing) = active {
            let before = writing.receipt(Ok(())).acknowledged_bytes;
            let mut outcome = writing.poll(input, &mut cx);
            progressed = writing.receipt(Ok(())).acknowledged_bytes != before;
            // Complete the final framing/flush checkpoint before observing a
            // peer that exits immediately after consuming the complete request.
            // At most two bounded write polls occur before the next read/stop.
            if progressed && outcome.is_pending() {
                outcome = writing.poll(input, &mut cx);
            }
            if let Poll::Ready(outcome) = outcome {
                let receipt = writing.receipt(outcome);
                settle(shared, &writing.response, receipt);
                *active = None;
                if receipt.attempted
                    && let Err(error) = receipt.outcome
                {
                    return error;
                }
            }
        }
        match read_once(output, &mut reader) {
            Ok(read) => progressed |= read,
            Err(error) => return error,
        }
        // Nonblocking pipes prevent server backpressure from hiding cancellation.
        // A fixed bounded poll quantum also limits ignored/empty-frame scanning.
        if !progressed {
            std::thread::sleep(POLL_INTERVAL);
        }
    }
}

fn settle(
    shared: &Shared,
    response: &Response<McpStdioWriteReceipt>,
    receipt: McpStdioWriteReceipt,
) {
    {
        let mut state = shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.admitted = state.admitted.saturating_sub(1);
    }
    response.complete(Ok(receipt));
}
fn prune(shared: &Shared, cx: &mut Context<'_>) -> Result<()> {
    // Move out before observing cancellation or destroying proofs/wakers.
    let queued = {
        let mut state = shared.state.lock().map_err(|_| McpStdioError::Closed)?;
        std::mem::take(&mut state.queue)
    };
    let mut retained = VecDeque::new();
    for queued in queued {
        let error = if queued.cancel.is_cancelled() {
            Some(McpStdioError::Cancelled)
        } else if Instant::now() >= queued.deadline {
            Some(McpStdioError::Deadline)
        } else if matches!(&queued.payload, Payload::Tool(submission) if submission.cancelled().as_mut().poll(cx).is_ready())
        {
            Some(McpStdioError::Cancelled)
        } else {
            None
        };
        if let Some(error) = error {
            settle(shared, &queued.response, failed(error));
        } else {
            retained.push_back(queued);
        }
    }
    let mut state = shared.state.lock().map_err(|_| McpStdioError::Closed)?;
    retained.append(&mut state.queue);
    state.queue = retained;
    Ok(())
}

fn read_once(output: &std::io::PipeReader, reader: &mut ReadState<'_>) -> Result<bool> {
    let shared = reader.shared;
    if shared
        .state
        .lock()
        .map_err(|_| McpStdioError::Closed)?
        .frames
        .len()
        >= MAX_MCP_STDIO_FRAMES
    {
        return Ok(false);
    }
    if reader.start == reader.end {
        match rustix::io::read(output, &mut reader.bytes) {
            Ok(0) => {
                return Err(if finalize_read(shared, &mut reader.decoder, true) {
                    McpStdioError::Protocol
                } else {
                    McpStdioError::Closed
                });
            }
            Ok(count) => {
                reader.start = 0;
                reader.end = count;
            }
            Err(rustix::io::Errno::AGAIN | rustix::io::Errno::INTR) => return Ok(false),
            Err(_) => return Err(McpStdioError::Process),
        }
    }
    let mut progressed = false;
    while reader.start < reader.end {
        if shared
            .state
            .lock()
            .map_err(|_| McpStdioError::Closed)?
            .frames
            .len()
            >= MAX_MCP_STDIO_FRAMES
        {
            break;
        }
        let progress = reader
            .decoder
            .push(&reader.bytes[reader.start..reader.end])
            .map_err(|_| McpStdioError::Protocol)?;
        reader.start += progress.consumed;
        progressed |= progress.consumed != 0;
        if let Some(frame) = progress.frame {
            shared
                .state
                .lock()
                .map_err(|_| McpStdioError::Closed)?
                .frames
                .push_back(frame);
            shared.reader.wake();
        }
    }
    Ok(progressed)
}

#[cfg(test)]
mod tests;
