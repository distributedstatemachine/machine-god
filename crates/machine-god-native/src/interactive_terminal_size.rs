//! Read-only dimensions from an explicitly retained presentation TTY.

use crate::{NativeOwnedWorkerCompletion, NativeOwnedWorkerScope};
use machine_god_core::BoxFuture;
use std::fmt;
use std::fs::File;
use std::num::NonZeroU16;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Fixed failures without descriptor, path or terminal-setting disclosure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeInteractiveTerminalSizeError {
    InvalidTerminal,
    InvalidColumns,
    InvalidRows,
    Busy,
    Unavailable,
}
impl fmt::Display for NativeInteractiveTerminalSizeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidTerminal => "terminal dimensions require a TTY",
            Self::InvalidColumns => "terminal columns are unavailable",
            Self::InvalidRows => "terminal rows are unavailable",
            Self::Busy => "terminal dimensions read is already active",
            Self::Unavailable => "terminal dimensions read is unavailable",
        })
    }
}
impl std::error::Error for NativeInteractiveTerminalSizeError {}
type Error = NativeInteractiveTerminalSizeError;

/// Nonzero columns and rows from one terminal-size observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeInteractiveTerminalDimensions {
    columns: NonZeroU16,
    rows: NonZeroU16,
}
impl NativeInteractiveTerminalDimensions {
    /// Pure validation without terminal access or guessed dimensions.
    ///
    /// # Errors
    /// Rejects zero columns or rows, checking columns first.
    pub fn new(columns: u16, rows: u16) -> Result<Self, Error> {
        Ok(Self {
            columns: NonZeroU16::new(columns).ok_or(Error::InvalidColumns)?,
            rows: NonZeroU16::new(rows).ok_or(Error::InvalidRows)?,
        })
    }

    #[must_use]
    pub const fn columns(self) -> NonZeroU16 {
        self.columns
    }

    #[must_use]
    pub const fn rows(self) -> NonZeroU16 {
        self.rows
    }
}

/// Retains the caller's stdout TTY without opening ambient paths or changing
/// terminal modes or status flags. Each read observes the current dimensions;
/// there is no resize subscription, cached default, or perpetual worker.
pub struct NativeInteractiveTerminalSizeReader {
    tty: Arc<File>,
    scope: NativeOwnedWorkerScope,
    active: Arc<AtomicBool>,
    io: Arc<dyn SizeIo>,
}
impl fmt::Debug for NativeInteractiveTerminalSizeReader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeInteractiveTerminalSizeReader")
            .finish_non_exhaustive()
    }
}
impl NativeInteractiveTerminalSizeReader {
    /// Inert: no descriptor validation, ioctl, worker or collector admission.
    #[must_use]
    pub fn new(stdout_tty: File) -> Self {
        Self {
            tty: Arc::new(stdout_tty),
            scope: NativeOwnedWorkerScope::new(),
            active: Arc::new(AtomicBool::new(false)),
            io: Arc::new(NativeSizeIo),
        }
    }

    /// Inert until first poll. At most one native read is admitted, including
    /// reads whose awaiting future was dropped. An admitted read stays owned
    /// through settlement; dropping its future does not cancel a native ioctl.
    ///
    /// # Errors
    /// Rejects non-TTY descriptors, zero columns, overlapping reads, and worker
    /// admission or native failures. No guessed column count is substituted.
    /// Zero rows do not invalidate this columns-only observation.
    pub fn read_columns(&mut self) -> BoxFuture<'_, Result<NonZeroU16, Error>> {
        Box::pin(async move {
            let size = self.read_size().await?;
            NonZeroU16::new(size.ws_col).ok_or(Error::InvalidColumns)
        })
    }

    /// Observes rows and columns together using one native size ioctl.
    /// Shares the same admission and owned-worker lifetime as `read_columns`.
    /// Construction is inert until the future is polled.
    ///
    /// # Errors
    /// Rejects non-TTY descriptors, zero columns or rows, overlapping reads,
    /// and worker admission or native failures. No defaults are substituted.
    pub fn read_dimensions(
        &mut self,
    ) -> BoxFuture<'_, Result<NativeInteractiveTerminalDimensions, Error>> {
        Box::pin(async move {
            let size = self.read_size().await?;
            NativeInteractiveTerminalDimensions::new(size.ws_col, size.ws_row)
        })
    }

    async fn read_size(&mut self) -> Result<rustix::termios::Winsize, Error> {
        self.active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::Busy)?;
        let permit = ReadPermit(self.active.clone());
        let tty = self.tty.clone();
        let io = self.io.clone();
        self.scope
            .run(move || {
                let _permit = permit;
                io.size(&tty)
            })
            .await
            .map_err(|_| Error::Unavailable)?
    }

    /// Permanently closes admission without abandoning an already owned read.
    pub fn close(&self) {
        self.scope.close();
    }

    /// Actual worker join, independent of whether a result was consumed.
    /// Close or drop the reader before awaiting completion.
    #[must_use]
    pub fn completion(&self) -> NativeOwnedWorkerCompletion {
        self.scope.completion()
    }
}
impl Drop for NativeInteractiveTerminalSizeReader {
    fn drop(&mut self) {
        self.close();
    }
}

struct ReadPermit(Arc<AtomicBool>);
impl Drop for ReadPermit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

trait SizeIo: Send + Sync {
    fn size(&self, tty: &File) -> Result<rustix::termios::Winsize, Error>;
}
struct NativeSizeIo;
impl SizeIo for NativeSizeIo {
    fn size(&self, tty: &File) -> Result<rustix::termios::Winsize, Error> {
        let stat = rustix::fs::fstat(tty).map_err(|_| Error::Unavailable)?;
        if rustix::fs::FileType::from_raw_mode(stat.st_mode)
            != rustix::fs::FileType::CharacterDevice
        {
            return Err(Error::InvalidTerminal);
        }
        rustix::termios::tcgetattr(tty).map_err(|_| Error::InvalidTerminal)?;
        rustix::termios::tcgetwinsize(tty).map_err(|_| Error::Unavailable)
    }
}

#[cfg(test)]
mod tests;
