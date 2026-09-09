//! Bounded presentation-side recording lane. Native owns every file write.

use super::OutputAcknowledgement;
use machine_god_core::{BoxFuture, CancellationToken};
use machine_god_native::{
    MAX_TERMINAL_TAPE_RECORDING_FRAME_BYTES, TerminalTapeRecorder, TerminalTapeRecordingError,
    TerminalTapeRecordingFrame, TerminalTapeRecordingStatus,
};
use std::{
    collections::VecDeque,
    task::{Context, Poll},
};

const MAX_QUEUED_EVENTS: usize = 8;

enum Event {
    Stdin(Vec<u8>),
    Stdout(Vec<u8>),
    Resize(u16, u16),
    Sigint,
    Marker(Vec<u8>),
}

impl Event {
    fn frame(&self) -> TerminalTapeRecordingFrame<'_> {
        match self {
            Self::Stdin(bytes) => TerminalTapeRecordingFrame::StdinAccepted(bytes),
            Self::Stdout(bytes) => TerminalTapeRecordingFrame::StdoutWritten(bytes),
            Self::Resize(cols, rows) => TerminalTapeRecordingFrame::Resize {
                cols: *cols,
                rows: *rows,
            },
            Self::Sigint => TerminalTapeRecordingFrame::Sigint,
            Self::Marker(bytes) => TerminalTapeRecordingFrame::Marker(bytes),
        }
    }
}

struct QueuedEvent {
    event: Event,
    timestamp_ms: i64,
}

struct StdoutReceipt {
    bytes: Vec<u8>,
    offset: usize,
    timestamp_ms: i64,
    waiting: bool,
    failed: bool,
}

struct Recorded {
    recorder: TerminalTapeRecorder,
    result: Result<TerminalTapeRecordingStatus, TerminalTapeRecordingError>,
    stdout: bool,
    finish: bool,
}

pub(crate) struct TapeLane {
    recorder: Option<TerminalTapeRecorder>,
    pending: Option<BoxFuture<'static, Recorded>>,
    queue: VecDeque<QueuedEvent>,
    stdout: Option<StdoutReceipt>,
    record_stdin: bool,
    failed: bool,
    finished: bool,
    clock: fn() -> Result<i64, ()>,
}

impl TapeLane {
    pub(crate) fn new(recorder: TerminalTapeRecorder, record_stdin: bool) -> Self {
        Self {
            recorder: Some(recorder),
            pending: None,
            queue: VecDeque::new(),
            stdout: None,
            record_stdin,
            failed: false,
            finished: false,
            clock: super::super::wall_clock_ms,
        }
    }

    pub(crate) fn stdin(&mut self, bytes: &[u8]) {
        if self.record_stdin && !bytes.is_empty() {
            if bytes.len() > MAX_TERMINAL_TAPE_RECORDING_FRAME_BYTES {
                self.abort();
            } else {
                self.enqueue(Event::Stdin(bytes.to_vec()));
            }
        }
    }

    pub(crate) fn resize(&mut self, cols: u16, rows: u16) {
        self.enqueue(Event::Resize(cols, rows));
    }

    pub(crate) fn sigint(&mut self) {
        self.enqueue(Event::Sigint);
    }

    pub(crate) fn marker(&mut self, bytes: &[u8]) {
        if bytes.len() > MAX_TERMINAL_TAPE_RECORDING_FRAME_BYTES {
            self.abort();
        } else {
            self.enqueue(Event::Marker(bytes.to_vec()));
        }
    }

    fn enqueue(&mut self, event: Event) {
        if self.failed || self.finished {
            return;
        }
        let Ok(timestamp_ms) = (self.clock)() else {
            self.abort();
            return;
        };
        if self.queue.len() == MAX_QUEUED_EVENTS {
            // Input and signals must not stop progressing behind recording.
            // Exhaustion is an explicit incomplete tape, never silent eviction.
            self.abort();
        } else {
            self.queue.push_back(QueuedEvent {
                event,
                timestamp_ms,
            });
        }
    }

    pub(super) fn stdout(&mut self, bytes: Vec<u8>, failed: bool) {
        let timestamp_ms = (self.clock)();
        if self.stdout.is_some() {
            self.abort();
            return;
        }
        self.stdout = Some(StdoutReceipt {
            bytes,
            offset: 0,
            timestamp_ms: timestamp_ms.unwrap_or(0),
            waiting: false,
            failed,
        });
        if timestamp_ms.is_err() {
            self.abort();
        }
    }

    /// None means there is no output receipt to acknowledge. The complete
    /// accepted prefix stays owned until recording succeeds or explicitly fails.
    pub(super) fn poll_stdout(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Option<Poll<OutputAcknowledgement>> {
        self.stdout.as_ref()?;
        if self.poll_progress(cx).is_err() {
            self.stdout.take();
            return Some(Poll::Ready(OutputAcknowledgement::Failed));
        }
        let stdout = self.stdout.as_mut().expect("retained receipt");
        if stdout.waiting {
            return Some(Poll::Pending);
        }
        if stdout.offset == stdout.bytes.len() {
            let stdout = self.stdout.take().expect("completed receipt");
            if stdout.failed {
                self.abort();
            }
            return Some(Poll::Ready(if stdout.failed {
                OutputAcknowledgement::Failed
            } else {
                OutputAcknowledgement::Succeeded
            }));
        }
        if self.queue.len() < MAX_QUEUED_EVENTS {
            let end = stdout
                .bytes
                .len()
                .min(stdout.offset + MAX_TERMINAL_TAPE_RECORDING_FRAME_BYTES);
            self.queue.push_back(QueuedEvent {
                event: Event::Stdout(stdout.bytes[stdout.offset..end].to_vec()),
                timestamp_ms: stdout.timestamp_ms,
            });
            stdout.offset = end;
            stdout.waiting = true;
            cx.waker().wake_by_ref();
        }
        Some(Poll::Pending)
    }

    pub(crate) fn poll_progress(&mut self, cx: &mut Context<'_>) -> Result<bool, ()> {
        if self.failed {
            return Err(());
        }
        if let Some(pending) = &mut self.pending {
            let Poll::Ready(recorded) = pending.as_mut().poll(cx) else {
                return Ok(false);
            };
            self.pending = None;
            self.recorder = Some(recorded.recorder);
            if recorded.result.is_err() {
                self.abort();
                return Err(());
            }
            if recorded.stdout {
                self.stdout
                    .as_mut()
                    .expect("admitted stdout receipt")
                    .waiting = false;
            }
            self.finished |= recorded.finish;
        }
        if let Some(queued) = self.queue.pop_front() {
            let Some(mut recorder) = self.recorder.take() else {
                self.abort();
                return Err(());
            };
            self.pending = Some(Box::pin(async move {
                let result = recorder
                    .record(
                        queued.event.frame(),
                        queued.timestamp_ms,
                        CancellationToken::new(),
                    )
                    .await;
                Recorded {
                    recorder,
                    result,
                    stdout: matches!(queued.event, Event::Stdout(_)),
                    finish: false,
                }
            }));
            cx.waker().wake_by_ref();
            return Ok(false);
        }
        Ok(self.stdout.is_none())
    }

    pub(crate) fn poll_finish(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), ()>> {
        match self.poll_progress(cx) {
            Err(()) => return Poll::Ready(Err(())),
            Ok(false) => return Poll::Pending,
            Ok(true) => {}
        }
        if self.finished {
            return Poll::Ready(Ok(()));
        }
        let Some(mut recorder) = self.recorder.take() else {
            self.abort();
            return Poll::Ready(Err(()));
        };
        self.pending = Some(Box::pin(async move {
            let result = recorder.finish().await;
            Recorded {
                recorder,
                result,
                stdout: false,
                finish: true,
            }
        }));
        cx.waker().wake_by_ref();
        Poll::Pending
    }

    /// Started native writes still finish and close under the retained scope.
    /// The startup owner observes the incomplete receipt and joins that scope.
    pub(crate) fn abort(&mut self) {
        self.failed = true;
        self.pending.take();
        self.recorder.take();
        self.queue.clear();
    }

    #[cfg(test)]
    pub(crate) fn hold_for_test(&mut self, until: tokio::sync::oneshot::Receiver<()>) {
        let recorder = self.recorder.take().expect("idle test recorder");
        self.pending = Some(Box::pin(async move {
            let _ = until.await;
            let result = Ok(recorder.completion().status());
            Recorded {
                recorder,
                result,
                stdout: false,
                finish: false,
            }
        }));
    }
}

#[cfg(test)]
mod tests;
