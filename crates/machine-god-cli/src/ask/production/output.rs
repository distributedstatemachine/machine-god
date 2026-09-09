//! Output transport shared by one-shot and interactive presentation drivers.

pub(super) enum OutputWork {
    Write(Vec<u8>),
    Flush,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum OutputAcknowledgement {
    Succeeded,
    Failed,
    /// The exact prefix accepted by stdout, including a failed partial write.
    Written {
        bytes: Vec<u8>,
        failed: bool,
        timestamp_ms: Result<i64, ()>,
    },
}

pub(super) struct OutputBridge {
    pub(super) work: tokio::sync::mpsc::Sender<OutputWork>,
    pub(super) acknowledgements: tokio::sync::mpsc::Receiver<OutputAcknowledgement>,
    pub(super) tape: Option<tape::TapeLane>,
}

pub(super) mod tape;

impl OutputBridge {
    /// A successful acknowledgement includes durable recording admission and
    /// completion for the actual stdout prefix, never the attempted suffix.
    pub(super) fn poll_acknowledgement(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<OutputAcknowledgement>> {
        use std::task::Poll;
        if let Some(tape) = &mut self.tape {
            if let Some(result) = tape.poll_stdout(cx) {
                return result.map(Some);
            }
            if tape.poll_progress(cx).is_err() {
                return Poll::Ready(Some(OutputAcknowledgement::Failed));
            }
        }
        let Poll::Ready(acknowledgement) = self.acknowledgements.poll_recv(cx) else {
            return Poll::Pending;
        };
        match acknowledgement {
            Some(OutputAcknowledgement::Written {
                bytes,
                failed,
                timestamp_ms,
            }) => {
                if let Some(tape) = &mut self.tape {
                    tape.stdout(bytes, failed, timestamp_ms);
                    tape.poll_stdout(cx)
                        .expect("stdout receipt is retained")
                        .map(Some)
                } else {
                    Poll::Ready(Some(if failed {
                        OutputAcknowledgement::Failed
                    } else {
                        OutputAcknowledgement::Succeeded
                    }))
                }
            }
            acknowledgement => Poll::Ready(acknowledgement),
        }
    }

    pub(super) async fn acknowledgement(&mut self) -> Option<OutputAcknowledgement> {
        std::future::poll_fn(|cx| self.poll_acknowledgement(cx)).await
    }

    pub(super) fn poll_tape(&mut self, cx: &mut std::task::Context<'_>) -> Result<bool, ()> {
        self.tape
            .as_mut()
            .map_or(Ok(true), |tape| tape.poll_progress(cx))
    }

    pub(super) fn poll_finish_tape(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), ()>> {
        self.tape
            .as_mut()
            .map_or(std::task::Poll::Ready(Ok(())), |tape| tape.poll_finish(cx))
    }

    pub(super) fn abort_tape(&mut self) {
        if let Some(tape) = &mut self.tape {
            tape.abort();
        }
    }
}

pub(super) fn serve_output(
    work: tokio::sync::mpsc::Receiver<OutputWork>,
    acknowledgements: &tokio::sync::mpsc::Sender<OutputAcknowledgement>,
    output: &mut dyn std::io::Write,
) {
    serve_output_with_clock(work, acknowledgements, output, super::wall_clock_ms);
}

fn serve_output_with_clock(
    mut work: tokio::sync::mpsc::Receiver<OutputWork>,
    acknowledgements: &tokio::sync::mpsc::Sender<OutputAcknowledgement>,
    output: &mut dyn std::io::Write,
    clock: fn() -> Result<i64, ()>,
) {
    while let Some(work) = work.blocking_recv() {
        let acknowledgement = match work {
            OutputWork::Write(mut bytes) => {
                let (accepted, failed) = write_prefix(output, &bytes);
                let timestamp_ms = clock();
                bytes.truncate(accepted);
                OutputAcknowledgement::Written {
                    bytes,
                    failed,
                    timestamp_ms,
                }
            }
            OutputWork::Flush => {
                if output.flush().is_ok() {
                    OutputAcknowledgement::Succeeded
                } else {
                    OutputAcknowledgement::Failed
                }
            }
        };
        if acknowledgements.blocking_send(acknowledgement).is_err() {
            break;
        }
    }
}

fn write_prefix(output: &mut dyn std::io::Write, bytes: &[u8]) -> (usize, bool) {
    let mut accepted = 0;
    // Bound zero-progress interruptions without limiting ordinary short writes.
    let mut interrupted = 0;
    while accepted < bytes.len() {
        match output.write(&bytes[accepted..]) {
            Ok(0) => return (accepted, true),
            Ok(count) if count <= bytes.len() - accepted => {
                accepted += count;
                interrupted = 0;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted && interrupted < 4096 => {
                interrupted += 1;
            }
            Ok(_) | Err(_) => return (accepted, true),
        }
    }
    (accepted, false)
}

#[cfg(test)]
pub(super) mod tests;
