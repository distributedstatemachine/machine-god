//! Presentation-host signal observation with explicitly owned native reads.

use machine_god_core::BoxFuture;
use machine_god_native::{NativeInteractiveTerminalSizeError, NativeInteractiveTerminalSizeReader};
use std::num::NonZeroU16;
use std::task::{Context, Poll};

type ReadResult = (
    NativeInteractiveTerminalSizeReader,
    Result<NonZeroU16, NativeInteractiveTerminalSizeError>,
);

pub(super) struct Resize {
    reader: Option<NativeInteractiveTerminalSizeReader>,
    pending: Option<BoxFuture<'static, ReadResult>>,
    signal: tokio::signal::unix::Signal,
    refresh: bool,
}

impl Resize {
    pub fn new(reader: NativeInteractiveTerminalSizeReader) -> Result<Self, ()> {
        Ok(Self {
            reader: Some(reader),
            pending: None,
            signal: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())
                .map_err(|_| ())?,
            refresh: false,
        })
    }

    /// Subscribe before reading, so a concurrent resize remains observable.
    pub async fn initial_columns(&mut self) -> Result<NonZeroU16, ()> {
        self.reader
            .as_mut()
            .ok_or(())?
            .read_columns()
            .await
            .map_err(|_| ())
    }

    pub fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Result<NonZeroU16, ()>> {
        match self.signal.poll_recv(cx) {
            Poll::Ready(Some(())) => self.refresh = true,
            Poll::Ready(None) => return Poll::Ready(Err(())),
            Poll::Pending => {}
        }
        if self.pending.is_none() && self.refresh {
            self.refresh = false;
            let mut reader = self.reader.take().expect("idle dimensions reader");
            self.pending = Some(Box::pin(async move {
                let result = reader.read_columns().await;
                (reader, result)
            }));
        }
        let Some(pending) = &mut self.pending else {
            return Poll::Pending;
        };
        let Poll::Ready((reader, columns)) = pending.as_mut().poll(cx) else {
            return Poll::Pending;
        };
        self.pending.take();
        self.reader = Some(reader);
        if self.refresh {
            cx.waker().wake_by_ref();
        }
        Poll::Ready(columns.map_err(|_| ()))
    }
}
