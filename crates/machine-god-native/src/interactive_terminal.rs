//! Explicit, single-lifetime raw-terminal authority with owned restoration.

use crate::{NativeOwnedWorkerCompletion, NativeOwnedWorkerScope};
use futures_util::future::poll_fn;
use machine_god_core::BoxFuture;
use rustix::termios::{
    ControlModes, InputModes, LocalModes, OptionalActions, SpecialCodeIndex, Termios,
};
use std::fmt;
use std::fs::File;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll, Waker};

/// Fixed failure: no descriptor, path, terminal settings or input disclosure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeInteractiveTerminalError {
    InvalidTerminal,
    Unavailable,
    ActivationFailed,
    RestoreFailed,
}
impl fmt::Display for NativeInteractiveTerminalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidTerminal => "interactive terminal descriptor is unsupported",
            Self::Unavailable => "interactive terminal is unavailable",
            Self::ActivationFailed => "interactive terminal activation failed",
            Self::RestoreFailed => "interactive terminal restoration failed",
        })
    }
}
impl std::error::Error for NativeInteractiveTerminalError {}
type Error = NativeInteractiveTerminalError;

/// A settled restoration observation, not merely an accepted cleanup request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeInteractiveTerminalRestoreReceipt {
    /// No terminal mutation was attempted.
    NotActivated,
    /// Original settings were installed and verified on the retained descriptor.
    Restored,
}
type Receipt = NativeInteractiveTerminalRestoreReceipt;
type RestoreResult = Result<Receipt, Error>;

#[derive(Default)]
struct State {
    activation: Option<Result<(), Error>>,
    restoration: Option<RestoreResult>,
    restore_requested: bool,
    waker: Option<Waker>,
}
#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}
impl Shared {
    fn requested(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .restore_requested
    }

    fn publish(&self, activation: Option<Result<(), Error>>, restoration: Option<RestoreResult>) {
        let waker = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.activation.is_none() {
                state.activation = activation;
            }
            if state.restoration.is_none() {
                state.restoration = restoration;
            }
            state.waker.take()
        };
        contain_waker(|| {
            if let Some(waker) = waker {
                waker.wake();
            }
        });
    }

    fn wait_for_restore(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !state.restore_requested {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}

/// Owns explicit exclusive termios authority, not ambient stdin. The caller
/// reserves terminal mode control against all aliases for this entire lifetime.
/// File ownership or duplication alone does not establish that authority.
/// Stop and join all input readers before requesting restoration or dropping
/// this guard. Activation/restoration never alter file status flags.
pub struct NativeInteractiveTerminal {
    source: Option<File>,
    shared: Arc<Shared>,
    scope: NativeOwnedWorkerScope,
    io: Arc<dyn TerminalIo>,
}
impl fmt::Debug for NativeInteractiveTerminal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeInteractiveTerminal")
            .finish_non_exhaustive()
    }
}
impl NativeInteractiveTerminal {
    /// Inert binding of the caller's descriptor and exclusive termios authority.
    #[must_use]
    pub fn new(tty: File) -> Self {
        Self {
            source: Some(tty),
            shared: Arc::new(Shared::default()),
            scope: NativeOwnedWorkerScope::new(),
            io: Arc::new(NativeIo),
        }
    }

    /// Inert until polled; admits one owned worker and verifies pinned raw mode.
    /// Dropping a started, unfinished activation requests restoration. Repeated
    /// activation while active is harmless; restoration permanently closes it.
    ///
    /// # Errors
    /// Reports invalid TTY authority, admission, activation or restoration failure.
    pub fn activate(&mut self) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(Activation {
            owner: self,
            started: false,
            finished: false,
        })
    }

    /// Inert until polled. Starts restoration and awaits its exact persistent
    /// result. Dropping this future does not abandon already requested cleanup.
    /// Callers additionally await [`Self::completion`] for actual thread join.
    ///
    /// # Errors
    /// A failed or unverified restore remains an error on every later call.
    pub fn restore(&mut self) -> BoxFuture<'_, RestoreResult> {
        Box::pin(async move {
            self.request_restore();
            if self.source.take().is_some() {
                self.shared.publish(
                    Some(Err(Error::Unavailable)),
                    Some(Ok(Receipt::NotActivated)),
                );
            }
            poll_fn(|cx| {
                let incoming = cx.waker().clone();
                let (result, old) = {
                    let mut state = self
                        .shared
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if let Some(result) = state.restoration {
                        (Poll::Ready(result), state.waker.take())
                    } else {
                        (Poll::Pending, state.waker.replace(incoming))
                    }
                };
                contain_waker(|| drop(old));
                result
            })
            .await
        })
    }

    /// Requests only this guard's restoration; no shared host token is changed.
    /// This is not a restored receipt and does not wait for a native syscall.
    pub fn request_restore(&self) {
        let waker = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.restore_requested = true;
            state.waker.take()
        };
        self.shared.changed.notify_all();
        self.scope.close();
        contain_waker(|| {
            if let Some(waker) = waker {
                waker.wake();
            }
        });
    }

    /// Actual owned collector join, including cleanup after dropped futures.
    /// Completion alone does not claim that restoration succeeded.
    #[must_use]
    pub fn completion(&self) -> NativeOwnedWorkerCompletion {
        self.scope.completion()
    }

    fn poll_activation(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        let incoming = cx.waker().clone();
        if self.shared.requested() {
            if self.source.take().is_some() {
                self.shared.publish(
                    Some(Err(Error::Unavailable)),
                    Some(Ok(Receipt::NotActivated)),
                );
            }
            return Poll::Ready(Err(Error::Unavailable));
        }
        if let Some(file) = self.source.take() {
            let shared = self.shared.clone();
            let scope = self.scope.clone();
            let io = self.io.clone();
            if self
                .scope
                .spawn(move || {
                    if let Err(payload) =
                        catch_unwind(AssertUnwindSafe(|| run_worker(file, &shared, &*io)))
                    {
                        std::mem::forget(payload);
                        shared
                            .publish(Some(Err(Error::Unavailable)), Some(Err(Error::Unavailable)));
                    }
                    scope.close();
                })
                .is_err()
            {
                self.shared.publish(
                    Some(Err(Error::Unavailable)),
                    Some(Ok(Receipt::NotActivated)),
                );
                self.scope.close();
            }
        }
        let (result, old) = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(result) = state.activation {
                (Poll::Ready(result), state.waker.take())
            } else {
                (Poll::Pending, state.waker.replace(incoming))
            }
        };
        contain_waker(|| drop(old));
        result
    }
}
impl Drop for NativeInteractiveTerminal {
    fn drop(&mut self) {
        self.request_restore();
    }
}

struct Activation<'a> {
    owner: &'a mut NativeInteractiveTerminal,
    started: bool,
    finished: bool,
}
impl Future for Activation<'_> {
    type Output = Result<(), Error>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.started = true;
        let result = self.owner.poll_activation(cx);
        self.finished = result.is_ready();
        result
    }
}
impl Drop for Activation<'_> {
    fn drop(&mut self) {
        if self.started && !self.finished {
            self.owner.request_restore();
        }
    }
}

trait TerminalIo: Send + Sync {
    fn get(&self, file: &File) -> Result<Termios, Error>;
    fn set(&self, file: &File, action: OptionalActions, settings: &Termios) -> Result<(), Error>;
}
struct NativeIo;
impl TerminalIo for NativeIo {
    fn get(&self, file: &File) -> Result<Termios, Error> {
        rustix::termios::tcgetattr(file).map_err(|_| Error::InvalidTerminal)
    }
    fn set(&self, file: &File, action: OptionalActions, settings: &Termios) -> Result<(), Error> {
        rustix::termios::tcsetattr(file, action, settings).map_err(|_| Error::Unavailable)
    }
}

fn pinned_raw(original: &Termios) -> Termios {
    let mut raw = original.clone();
    raw.input_modes.remove(
        InputModes::BRKINT
            | InputModes::ICRNL
            | InputModes::INPCK
            | InputModes::ISTRIP
            | InputModes::IXON
            | InputModes::IXOFF,
    );
    raw.control_modes.remove(ControlModes::CSIZE);
    raw.control_modes.insert(ControlModes::CS8);
    raw.local_modes
        .remove(LocalModes::ECHO | LocalModes::ICANON | LocalModes::IEXTEN | LocalModes::ISIG);
    raw.special_codes[SpecialCodeIndex::VMIN] = 1;
    raw.special_codes[SpecialCodeIndex::VTIME] = 0;
    raw
}

fn same_settings(left: &Termios, right: &Termios) -> bool {
    // rustix exposes no equality/iteration for SpecialCodes. Its pinned Debug
    // rendering covers every fixed NCCS byte; no terminal input is formatted.
    format!("{left:?}") == format!("{right:?}")
}

struct RestoreOnDrop<'a> {
    file: File,
    original: Termios,
    io: &'a dyn TerminalIo,
    attempted: bool,
    settled: bool,
}
impl RestoreOnDrop<'_> {
    fn restore(&mut self) -> RestoreResult {
        self.settled = true;
        if !self.attempted {
            return Ok(Receipt::NotActivated);
        }
        self.io
            .set(&self.file, OptionalActions::Flush, &self.original)
            .map_err(|_| Error::RestoreFailed)?;
        let current = self.io.get(&self.file).map_err(|_| Error::RestoreFailed)?;
        if !same_settings(&self.original, &current) {
            return Err(Error::RestoreFailed);
        }
        Ok(Receipt::Restored)
    }
}
impl Drop for RestoreOnDrop<'_> {
    fn drop(&mut self) {
        if !self.settled {
            // Best effort on unwind. Opaque panic payload destructors are not
            // allowed to interrupt descriptor ownership or collector joining.
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
                let _ = self.restore();
            })) {
                std::mem::forget(payload);
            }
        }
    }
}

fn run_worker(file: File, shared: &Shared, io: &dyn TerminalIo) {
    let original = match rustix::fs::fstat(&file) {
        Ok(stat)
            if rustix::fs::FileType::from_raw_mode(stat.st_mode)
                == rustix::fs::FileType::CharacterDevice =>
        {
            io.get(&file)
        }
        _ => Err(Error::InvalidTerminal),
    };
    let original = match original {
        Ok(settings) => settings,
        Err(error) => {
            shared.publish(Some(Err(error)), Some(Ok(Receipt::NotActivated)));
            return;
        }
    };
    let raw = pinned_raw(&original);
    let mut owned = RestoreOnDrop {
        file,
        original,
        io,
        attempted: false,
        settled: false,
    };
    if shared.requested() {
        shared.publish(Some(Err(Error::Unavailable)), Some(owned.restore()));
        return;
    }
    owned.attempted = true;
    let activation = io
        .set(&owned.file, OptionalActions::Now, &raw)
        .and_then(|()| io.get(&owned.file))
        .map(|current| same_settings(&raw, &current))
        .unwrap_or(false);
    if !activation {
        let result = owned.restore();
        shared.publish(
            Some(Err(if result.is_err() {
                Error::RestoreFailed
            } else {
                Error::ActivationFailed
            })),
            Some(result),
        );
        return;
    }
    shared.publish(Some(Ok(())), None);
    shared.wait_for_restore();
    shared.publish(None, Some(owned.restore()));
}

fn contain_waker(operation: impl FnOnce()) {
    if let Err(payload) = catch_unwind(AssertUnwindSafe(operation)) {
        std::mem::forget(payload);
    }
}

#[cfg(test)]
mod tests;
