//! Presentation-host signal observation with explicitly owned native reads.

use machine_god_core::BoxFuture;
use machine_god_native::{
    NativeInteractiveTerminalDimensions, NativeInteractiveTerminalSizeError,
    NativeInteractiveTerminalSizeReader,
};
use std::task::{Context, Poll};

type ReadResult = (
    NativeInteractiveTerminalSizeReader,
    Result<NativeInteractiveTerminalDimensions, NativeInteractiveTerminalSizeError>,
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
    pub async fn initial_dimensions(&mut self) -> Result<NativeInteractiveTerminalDimensions, ()> {
        self.reader
            .as_mut()
            .ok_or(())?
            .read_dimensions()
            .await
            .map_err(|_| ())
    }

    pub fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<NativeInteractiveTerminalDimensions, ()>> {
        match self.signal.poll_recv(cx) {
            Poll::Ready(Some(())) => self.refresh = true,
            Poll::Ready(None) => return Poll::Ready(Err(())),
            Poll::Pending => {}
        }
        if self.pending.is_none() && self.refresh {
            self.refresh = false;
            let mut reader = self.reader.take().expect("idle dimensions reader");
            self.pending = Some(Box::pin(async move {
                let result = reader.read_dimensions().await;
                (reader, result)
            }));
        }
        let Some(pending) = &mut self.pending else {
            return Poll::Pending;
        };
        let Poll::Ready((reader, dimensions)) = pending.as_mut().poll(cx) else {
            return Poll::Pending;
        };
        self.pending.take();
        self.reader = Some(reader);
        if self.refresh {
            cx.waker().wake_by_ref();
        }
        Poll::Ready(dimensions.map_err(|_| ()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustix::fs::{Mode, OFlags};
    use std::{fs::File, future::poll_fn, task::Waker, time::Duration};

    fn pty() -> (File, File) {
        let master = rustix::fs::open(
            "/dev/ptmx",
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .unwrap();
        rustix::pty::grantpt(&master).unwrap();
        rustix::pty::unlockpt(&master).unwrap();
        #[cfg(target_os = "linux")]
        let slave = rustix::pty::ioctl_tiocgptpeer(
            &master,
            rustix::pty::OpenptFlags::RDWR
                | rustix::pty::OpenptFlags::NOCTTY
                | rustix::pty::OpenptFlags::CLOEXEC,
        )
        .unwrap();
        #[cfg(target_os = "macos")]
        let slave = {
            let name = rustix::pty::ptsname(&master, Vec::new()).unwrap();
            rustix::fs::open(
                name.as_c_str(),
                OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .unwrap()
        };
        (master.into(), slave.into())
    }

    fn set_dimensions(file: &File, columns: u16, rows: u16) {
        rustix::termios::tcsetwinsize(
            file,
            rustix::termios::Winsize {
                ws_col: columns,
                ws_row: rows,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )
        .unwrap();
    }

    #[test]
    fn initial_and_coalesced_refresh_observe_real_rows_and_columns() {
        let runtime = machine_god_native::TokioWebSearchDeadline::build_runtime_pair()
            .unwrap()
            .0;
        let (master, slave) = pty();
        let reader = NativeInteractiveTerminalSizeReader::new(slave);
        let completion = reader.completion();
        runtime.block_on(async {
            let mut resize = Resize::new(reader).unwrap();
            set_dimensions(&master, 80, 24);
            assert_eq!(
                resize.initial_dimensions().await.unwrap(),
                NativeInteractiveTerminalDimensions::new(80, 24).unwrap()
            );
            set_dimensions(&master, 90, 30);
            resize.refresh = true;
            set_dimensions(&master, 120, 50);
            resize.refresh = true;
            let observed =
                tokio::time::timeout(Duration::from_secs(10), poll_fn(|cx| resize.poll(cx)))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(
                observed,
                NativeInteractiveTerminalDimensions::new(120, 50).unwrap()
            );
            assert!(resize.pending.is_none());
            assert!(resize.reader.is_some());
            set_dimensions(&master, 80, 0);
            resize.refresh = true;
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(10), poll_fn(|cx| resize.poll(cx)))
                    .await
                    .unwrap(),
                Err(())
            );
            set_dimensions(&master, 100, 60);
            resize.refresh = true;
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(10), poll_fn(|cx| resize.poll(cx)))
                    .await
                    .unwrap()
                    .unwrap(),
                NativeInteractiveTerminalDimensions::new(100, 60).unwrap()
            );
            drop(resize);
        });
        completion.wait_on_worker().unwrap();
        assert!(completion.is_complete());
    }

    #[test]
    fn dropping_resize_after_read_admission_preserves_owned_completion() {
        let runtime = machine_god_native::TokioWebSearchDeadline::build_runtime_pair()
            .unwrap()
            .0;
        let (master, slave) = pty();
        set_dimensions(&master, 80, 24);
        let reader = NativeInteractiveTerminalSizeReader::new(slave);
        let completion = reader.completion();
        runtime.block_on(async {
            let mut resize = Resize::new(reader).unwrap();
            resize.refresh = true;
            let _ = resize.poll(&mut Context::from_waker(Waker::noop()));
            drop(resize);
        });
        completion.wait_on_worker().unwrap();
        assert!(completion.is_complete());
    }
}
