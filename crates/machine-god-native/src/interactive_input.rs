//! Explicit, bounded, collector-owned interactive byte input on Linux/macOS.

use crate::{NativeOwnedWorkerCompletion, NativeOwnedWorkerScope};
use machine_god_core::CancellationToken;
use rustix::fs::{FileType, OFlags};
use std::fmt;
use std::fs::File;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

mod shared_input;
pub use shared_input::{
    INTERACTIVE_INPUT_HELPER_ARGUMENT, NativeInteractiveInputHelper, run_interactive_input_helper,
};

/// Maximum bytes in one input chunk. No line or UTF-8 framing is performed.
pub const NATIVE_INTERACTIVE_INPUT_CHUNK_BYTES: usize = 4096;
const WAIT_INTERVAL: Duration = Duration::from_millis(25);

/// Explicit input authority. Owning a `File` does not establish exclusive
/// ownership of its shared open-file description or its status flags.
#[derive(Default)]
pub enum NativeInteractiveInputSource {
    /// No input authority, reads, descriptor inspection or worker admission.
    #[default]
    Disabled,
    /// Require existing `O_NONBLOCK`; never change status flags.
    PreserveNonblocking(File),
    /// Explicitly authorize setting `O_NONBLOCK` on the shared open-file
    /// description, including every duplicated alias. Never restore flags.
    /// This is not automatically suitable for a caller's or shell's stdin.
    AdoptNonblockingStatus(File),
    /// Preserve inherited status flags: reopen a verified TTY independently,
    /// or use an explicitly owned helper for a blocking pipe. Acquisition and
    /// helper admission occur only on the first input poll.
    PreserveShared {
        input: File,
        helper: NativeInteractiveInputHelper,
    },
}

impl fmt::Debug for NativeInteractiveInputSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Disabled => "Disabled",
            Self::PreserveNonblocking(_) => "PreserveNonblocking(..)",
            Self::AdoptNonblockingStatus(_) => "AdoptNonblockingStatus(..)",
            Self::PreserveShared { .. } => "PreserveShared(..)",
        })
    }
}

/// Fixed failure without descriptor numbers, paths or input contents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeInteractiveInputError {
    InvalidDescriptor,
    NonblockingRequired,
    Unavailable,
    Read,
    Cancelled,
}

impl fmt::Display for NativeInteractiveInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidDescriptor => "interactive input descriptor is unsupported",
            Self::NonblockingRequired => "interactive input requires nonblocking authority",
            Self::Unavailable => "interactive input is unavailable",
            Self::Read => "interactive input read failed",
            Self::Cancelled => "interactive input was cancelled",
        })
    }
}
impl std::error::Error for NativeInteractiveInputError {}

/// Exact raw bytes, including partial code points and line delimiters.
pub struct NativeInteractiveInputChunk {
    bytes: [u8; NATIVE_INTERACTIVE_INPUT_CHUNK_BYTES],
    len: usize,
}
impl NativeInteractiveInputChunk {
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}
impl fmt::Debug for NativeInteractiveInputChunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeInteractiveInputChunk")
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

type Outcome = Result<(), NativeInteractiveInputError>;

#[derive(Default)]
struct State {
    demand: bool,
    chunk: Option<NativeInteractiveInputChunk>,
    terminal: Option<Outcome>,
    waker: Option<Waker>,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    stopped: AtomicBool,
    host_stop: CancellationToken,
}
impl Shared {
    fn wait_for_demand(&self) -> Outcome {
        loop {
            if self.cancelled() {
                return Err(NativeInteractiveInputError::Cancelled);
            }
            if self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .demand
            {
                return Ok(());
            }
            self.pause();
        }
    }

    fn publish_chunk(&self, chunk: NativeInteractiveInputChunk) -> Outcome {
        let waker = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.cancelled() {
                return Err(NativeInteractiveInputError::Cancelled);
            }
            state.chunk = Some(chunk);
            state.demand = false;
            state.waker.take()
        };
        wake(waker);
        Ok(())
    }

    fn cancelled(&self) -> bool {
        self.stopped.load(Ordering::Acquire) || self.host_stop.is_cancelled()
    }

    fn finish(&self, outcome: Outcome) {
        let waker = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.terminal.is_none() {
                state.terminal = Some(outcome);
            }
            state.chunk = None;
            state.demand = false;
            state.waker.take()
        };
        self.changed.notify_all();
        wake(waker);
    }

    fn pause(&self) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.cancelled() {
            drop(
                self.changed
                    .wait_timeout(state, WAIT_INTERVAL)
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
        }
    }
}

/// Owns one demand-gated input worker and one bounded result slot.
///
/// The caller reserves input consumption and status-flag control for this
/// adapter's lifetime: no alias may clear `O_NONBLOCK` or change termios. This semantic authority
/// cannot be inferred from `File` ownership or duplication. Only readable pipes
/// and verified TTYs are supported, not arbitrary filesystem or device reads.
pub struct NativeInteractiveInput {
    source: Option<NativeInteractiveInputSource>,
    shared: Arc<Shared>,
    scope: NativeOwnedWorkerScope,
    exhausted: bool,
}
impl Default for NativeInteractiveInput {
    fn default() -> Self {
        Self::new(
            NativeInteractiveInputSource::Disabled,
            CancellationToken::new(),
        )
    }
}
impl fmt::Debug for NativeInteractiveInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeInteractiveInput")
            .finish_non_exhaustive()
    }
}
impl NativeInteractiveInput {
    /// Inert: does not inspect descriptors, change flags, read, or start workers.
    #[must_use]
    pub fn new(source: NativeInteractiveInputSource, host_stop: CancellationToken) -> Self {
        Self {
            source: Some(source),
            shared: Arc::new(Shared {
                state: Mutex::new(State::default()),
                changed: Condvar::new(),
                stopped: AtomicBool::new(false),
                host_stop,
            }),
            scope: NativeOwnedWorkerScope::new(),
            exhausted: false,
        }
    }

    /// First poll admits the worker. Each pending poll authorizes at most one
    /// chunk; consuming it does not authorize another read until the next poll.
    /// Terminal errors are emitted once, followed by permanent exhaustion.
    ///
    /// # Errors
    /// Returns a fixed error for unsupported input, missing nonblocking
    /// authority, worker admission, native read/poll failure, or cancellation.
    pub fn poll_chunk(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<NativeInteractiveInputChunk>, NativeInteractiveInputError>> {
        if self.exhausted {
            return Poll::Ready(Ok(None));
        }
        if self.shared.cancelled() {
            self.source.take();
            self.shared
                .finish(Err(NativeInteractiveInputError::Cancelled));
            self.scope.close();
        }
        // Clone outside the lock, before admitting a worker. User wakers may panic.
        let incoming = cx.waker().clone();
        let (result, old_waker) = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(outcome) = state.terminal.take() {
                self.exhausted = true;
                (Poll::Ready(outcome.map(|()| None)), state.waker.take())
            } else if let Some(chunk) = state.chunk.take() {
                (Poll::Ready(Ok(Some(chunk))), state.waker.take())
            } else {
                state.demand = true;
                (Poll::Pending, state.waker.replace(incoming))
            }
        };
        discard_waker(old_waker);
        if result.is_ready() {
            return result;
        }
        self.shared.changed.notify_all();
        if let Some(source) = self.source.take() {
            if matches!(source, NativeInteractiveInputSource::Disabled) {
                self.shared.finish(Ok(()));
                self.scope.close();
            } else {
                let shared = Arc::clone(&self.shared);
                let scope = self.scope.clone();
                if self
                    .scope
                    .spawn(move || {
                        let outcome = catch_unwind(AssertUnwindSafe(|| {
                            run_worker(source, &shared, &NativeIo)
                        }));
                        let outcome = outcome.unwrap_or_else(|payload| {
                            std::mem::forget(payload);
                            Err(NativeInteractiveInputError::Unavailable)
                        });
                        shared.finish(outcome);
                        scope.close();
                    })
                    .is_err()
                {
                    self.shared
                        .finish(Err(NativeInteractiveInputError::Unavailable));
                    self.scope.close();
                }
            }
        }
        Poll::Pending
    }

    /// Stops only this adapter, never its caller's shared host token. Completion
    /// remains independently observable even without consuming pending input.
    pub fn request_stop(&self) {
        self.shared.stopped.store(true, Ordering::Release);
        self.shared.changed.notify_all();
        self.scope.close();
    }

    /// Actual collector join observation, not merely input EOF or cancellation.
    #[must_use]
    pub fn completion(&self) -> NativeOwnedWorkerCompletion {
        self.scope.completion()
    }
}
impl Drop for NativeInteractiveInput {
    fn drop(&mut self) {
        self.request_stop();
    }
}

fn discard_waker(waker: Option<Waker>) {
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(waker))) {
        std::mem::forget(payload);
    }
}
fn wake(waker: Option<Waker>) {
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
        if let Some(waker) = waker {
            waker.wake();
        }
    })) {
        std::mem::forget(payload);
    }
}

struct PreparedDescriptor {
    file: File,
    timed_tty: bool,
    empty_read_is_idle: bool,
}

fn prepare(
    source: NativeInteractiveInputSource,
) -> Result<PreparedDescriptor, NativeInteractiveInputError> {
    let (file, adopt) = match source {
        NativeInteractiveInputSource::Disabled
        | NativeInteractiveInputSource::PreserveShared { .. } => {
            return Err(NativeInteractiveInputError::InvalidDescriptor);
        }
        NativeInteractiveInputSource::PreserveNonblocking(file) => (file, false),
        NativeInteractiveInputSource::AdoptNonblockingStatus(file) => (file, true),
    };
    let stat =
        rustix::fs::fstat(&file).map_err(|_| NativeInteractiveInputError::InvalidDescriptor)?;
    let kind = FileType::from_raw_mode(stat.st_mode);
    let termios = if kind == FileType::CharacterDevice {
        Some(
            rustix::termios::tcgetattr(&file)
                .map_err(|_| NativeInteractiveInputError::InvalidDescriptor)?,
        )
    } else {
        None
    };
    let tty = termios.is_some();
    if kind != FileType::Fifo && !tty {
        return Err(NativeInteractiveInputError::InvalidDescriptor);
    }
    let flags = rustix::fs::fcntl_getfl(&file)
        .map_err(|_| NativeInteractiveInputError::InvalidDescriptor)?;
    if flags.contains(OFlags::WRONLY) {
        return Err(NativeInteractiveInputError::InvalidDescriptor);
    }
    if !flags.contains(OFlags::NONBLOCK) {
        if !adopt {
            return Err(NativeInteractiveInputError::NonblockingRequired);
        }
        rustix::fs::fcntl_setfl(&file, flags | OFlags::NONBLOCK)
            .map_err(|_| NativeInteractiveInputError::Unavailable)?;
    }
    let empty_read_is_idle = termios.is_some_and(|settings| {
        !settings
            .local_modes
            .contains(rustix::termios::LocalModes::ICANON)
            && settings.special_codes[rustix::termios::SpecialCodeIndex::VMIN] == 0
    });
    Ok(PreparedDescriptor {
        file,
        timed_tty: cfg!(target_os = "macos") && tty,
        empty_read_is_idle,
    })
}

trait InputIo {
    fn readable(&self, file: &File) -> rustix::io::Result<bool>;
    fn read(&self, file: &File, buffer: &mut [u8]) -> rustix::io::Result<usize>;
}
struct NativeIo;
impl InputIo for NativeIo {
    fn readable(&self, file: &File) -> rustix::io::Result<bool> {
        use rustix::event::{PollFd, PollFlags, Timespec, poll};
        let mut descriptors = [PollFd::new(file, PollFlags::IN)];
        poll(
            &mut descriptors,
            Some(&Timespec::try_from(WAIT_INTERVAL).expect("bounded duration")),
        )?;
        let events = descriptors[0].revents();
        if events.intersects(PollFlags::NVAL | PollFlags::ERR) {
            return Err(rustix::io::Errno::IO);
        }
        Ok(events.intersects(PollFlags::IN | PollFlags::HUP))
    }
    fn read(&self, file: &File, buffer: &mut [u8]) -> rustix::io::Result<usize> {
        rustix::io::read(file, buffer)
    }
}

fn run_worker(source: NativeInteractiveInputSource, shared: &Shared, io: &impl InputIo) -> Outcome {
    if shared.cancelled() {
        return Err(NativeInteractiveInputError::Cancelled);
    }
    let source = match source {
        NativeInteractiveInputSource::PreserveShared { input, helper } => {
            match shared_input::acquire(input, helper, shared)? {
                shared_input::AcquiredInput::Direct(file) => {
                    NativeInteractiveInputSource::PreserveNonblocking(file)
                }
                shared_input::AcquiredInput::Helper(helper) => return helper.drive(shared),
            }
        }
        source => source,
    };
    let PreparedDescriptor {
        file,
        timed_tty,
        empty_read_is_idle,
    } = prepare(source)?;
    loop {
        shared.wait_for_demand()?;
        if !timed_tty {
            match io.readable(&file) {
                Ok(true) => {}
                Ok(false) | Err(rustix::io::Errno::INTR) => continue,
                Err(_) => return Err(NativeInteractiveInputError::Read),
            }
        }
        if shared.cancelled() {
            return Err(NativeInteractiveInputError::Cancelled);
        }
        let mut chunk = NativeInteractiveInputChunk {
            bytes: [0; NATIVE_INTERACTIVE_INPUT_CHUNK_BYTES],
            len: 0,
        };
        match io.read(&file, &mut chunk.bytes) {
            Ok(0) if empty_read_is_idle => {
                shared.pause();
                continue;
            }
            Ok(0) => return Ok(()),
            Ok(len) => chunk.len = len,
            Err(rustix::io::Errno::INTR) => continue,
            Err(rustix::io::Errno::AGAIN) => {
                shared.pause();
                continue;
            }
            Err(_) => return Err(NativeInteractiveInputError::Read),
        }
        shared.publish_chunk(chunk)?;
    }
}

#[cfg(test)]
mod tests;
