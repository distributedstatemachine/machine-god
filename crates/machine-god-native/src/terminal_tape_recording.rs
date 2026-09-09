//! Explicit, bounded FXTP v1 recording owned by the native worker collector.

use crate::{FileSessionStore, NativeOwnedWorkerCompletion, NativeOwnedWorkerScope};
use machine_god_core::{BoxFuture, CancellationToken};
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};

mod filesystem;
mod reply;

/// Maximum payload retained by one admitted frame.
pub const MAX_TERMINAL_TAPE_RECORDING_FRAME_BYTES: usize = 64 * 1024;
/// Maximum number of complete recorded frames.
pub const MAX_TERMINAL_TAPE_RECORDING_FRAMES: u64 = 1_000_000;
/// Inclusive recording limit, strictly below the replay reader's input limit.
pub const MAX_TERMINAL_TAPE_RECORDING_BYTES: u64 = 64 * 1024 * 1024 - 1;
const MAX_WRITE_ATTEMPTS: usize = 4096;

/// Explicit destination authority. Automatic recording retains the selected
/// store, not a pathname-based rediscovery of the current profile.
pub enum TerminalTapeRecordingDestination {
    /// Create one private exclusive tape beneath this retained store.
    Automatic {
        /// Selected native store, whose descriptor is cloned on the worker.
        store: Arc<FileSessionStore>,
        /// Logical path used only to label the returned recording path.
        state_path: PathBuf,
    },
    /// Create an exclusive file at an explicit absolute path. Existing files
    /// and symlinks are rejected; missing parent directories are not created.
    Explicit(PathBuf),
}

impl fmt::Debug for TerminalTapeRecordingDestination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Automatic { .. } => "Automatic { .. }",
            Self::Explicit(_) => "Explicit { .. }",
        })
    }
}

/// Bounded, injected header/configuration. No environment or clock is consulted.
#[derive(Clone, Debug)]
pub struct TerminalTapeRecordingOptions {
    /// Initial terminal columns.
    pub cols: u16,
    /// Initial terminal rows.
    pub rows: u16,
    /// Injected epoch and initial frame timestamp in milliseconds.
    pub epoch_ms: i64,
    /// Header version bytes, at most 255; never silently truncated.
    pub version: Vec<u8>,
    /// Explicitly opt into recording accepted stdin bytes; false by default.
    pub record_stdin: bool,
    /// Inclusive tape-byte limit, no greater than the public hard cap.
    pub max_bytes: u64,
    /// Inclusive complete-frame limit, no greater than the public hard cap.
    pub max_frames: u64,
}

impl TerminalTapeRecordingOptions {
    /// Constructs ordinary recording bounds. Validation happens before startup
    /// effects, so callers can construct inert requests with invalid inputs.
    #[must_use]
    pub fn new(cols: u16, rows: u16, epoch_ms: i64, version: Vec<u8>) -> Self {
        Self {
            cols,
            rows,
            epoch_ms,
            version,
            record_stdin: false,
            max_bytes: MAX_TERMINAL_TAPE_RECORDING_BYTES,
            max_frames: MAX_TERMINAL_TAPE_RECORDING_FRAMES,
        }
    }

    fn validate(&self) -> Result<(), TerminalTapeRecordingError> {
        if !valid_dimensions(self.cols, self.rows)
            || self.version.len() > 255
            || self.max_bytes < 18 + self.version.len() as u64
            || self.max_bytes > MAX_TERMINAL_TAPE_RECORDING_BYTES
            || self.max_frames == 0
            || self.max_frames > MAX_TERMINAL_TAPE_RECORDING_FRAMES
        {
            return Err(TerminalTapeRecordingError::InvalidRequest);
        }
        Ok(())
    }
}

/// Caller-selected recording. Construction and an unpolled start are inert.
#[derive(Debug)]
pub struct TerminalTapeRecordingRequest {
    /// Explicit destination authority.
    pub destination: TerminalTapeRecordingDestination,
    /// Header, privacy choice and bounded resource configuration.
    pub options: TerminalTapeRecordingOptions,
}

/// Fixed recording errors; no path, terminal content or native diagnostic leaks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TerminalTapeRecordingError {
    /// Invalid path/header/dimensions/bounds, before startup effects.
    InvalidRequest,
    /// The platform has no supported confined recording implementation.
    UnsupportedPlatform,
    /// A destination could not be opened privately and exclusively.
    OpenFailed,
    /// The fixed automatic-name collision allowance was exhausted.
    AlreadyExists,
    /// A native write failed or made no progress.
    WriteFailed,
    /// File flush or durable sync failed.
    FlushFailed,
    /// A hard file/frame/payload/native-call limit was reached.
    LimitExceeded,
    /// Cancellation was observed before admission or between native calls.
    Cancelled,
    /// No frame was admitted; retain its bytes and retry after progress.
    Busy,
    /// The recording is already closing, closed or failed.
    Closed,
    /// Native worker admission or an unexpected worker failure.
    WorkerUnavailable,
    /// Recorder ownership ended without explicit successful finish.
    Abandoned,
}

impl fmt::Display for TerminalTapeRecordingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "terminal recording failed: {self:?}")
    }
}
impl std::error::Error for TerminalTapeRecordingError {}

/// Metadata-only progress and finalization receipt. Counts include exactly
/// acknowledged native write bytes, including a partially written final frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalTapeRecordingStatus {
    /// File/header setup succeeded and admission has not ended.
    pub active: bool,
    /// The file writer has actually been dropped after flush/sync attempts.
    pub closed: bool,
    /// The file completed a requested finish without any earlier error.
    pub complete: bool,
    /// Complete frame count; a torn last frame is not included.
    pub frames: u64,
    /// Acknowledged bytes, including header and any partial final frame.
    pub bytes_written: u64,
    /// First failure, retained through cleanup and later rejected requests.
    pub failure: Option<TerminalTapeRecordingError>,
}

impl TerminalTapeRecordingStatus {
    const fn pending() -> Self {
        Self {
            active: false,
            closed: false,
            complete: false,
            frames: 0,
            bytes_written: 0,
            failure: None,
        }
    }
}

/// Observation-only recording progress; does not keep the recorder or worker
/// alive. The enclosing host's completion proves collector join after closure.
#[derive(Clone, Debug)]
pub struct TerminalTapeRecordingCompletion {
    status: Arc<Mutex<TerminalTapeRecordingStatus>>,
    workers: NativeOwnedWorkerCompletion,
}

impl TerminalTapeRecordingCompletion {
    /// Returns current metadata without any I/O or worker admission.
    #[must_use]
    pub fn status(&self) -> TerminalTapeRecordingStatus {
        *self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Host-owned close-and-join fence, distinct from a file-close receipt.
    #[must_use]
    pub fn workers(&self) -> NativeOwnedWorkerCompletion {
        self.workers.clone()
    }
}

/// Bytes supplied here must already have been accepted by the real output or
/// input boundary. The recorder never attempts terminal I/O itself.
#[derive(Clone, Copy)]
pub enum TerminalTapeRecordingFrame<'a> {
    /// Exact stdout prefix accepted by the terminal writer.
    StdoutWritten(&'a [u8]),
    /// Accepted stdin, recorded only when the explicit input option is enabled.
    StdinAccepted(&'a [u8]),
    /// Accepted new terminal dimensions.
    Resize { cols: u16, rows: u16 },
    /// Observed SIGINT.
    Sigint,
    /// Explicit marker bytes.
    Marker(&'a [u8]),
}

impl fmt::Debug for TerminalTapeRecordingFrame<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::StdoutWritten(_) => "StdoutWritten { .. }",
            Self::StdinAccepted(_) => "StdinAccepted { .. }",
            Self::Resize { .. } => "Resize { .. }",
            Self::Sigint => "Sigint",
            Self::Marker(_) => "Marker { .. }",
        })
    }
}

/// Owns admission, not the file. Dropping it disconnects the bounded channel;
/// already admitted writes and subsequent flush/close remain scoped native work.
pub struct TerminalTapeRecorder {
    sender: Option<mpsc::SyncSender<Command>>,
    path: PathBuf,
    record_stdin: bool,
    completion: TerminalTapeRecordingCompletion,
}

impl fmt::Debug for TerminalTapeRecorder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalTapeRecorder")
            .field("status", &self.completion.status())
            .finish_non_exhaustive()
    }
}

impl TerminalTapeRecorder {
    /// Opens the tape and writes its complete header before returning success.
    /// The future is inert before poll. A dropped start disconnects admission;
    /// any already started file work still closes on the enrolled worker.
    pub fn start(
        request: TerminalTapeRecordingRequest,
        scope: NativeOwnedWorkerScope,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<Self, TerminalTapeRecordingError>> {
        Box::pin(async move {
            request.options.validate()?;
            check_cancelled(&cancellation)?;
            let status = Arc::new(Mutex::new(TerminalTapeRecordingStatus::pending()));
            let completion = TerminalTapeRecordingCompletion {
                status: Arc::clone(&status),
                workers: scope.completion(),
            };
            let (sender, receiver) = mpsc::sync_channel(1);
            let (startup, response) = reply::channel();
            let record_stdin = request.options.record_stdin;
            scope
                .spawn(move || {
                    run_recording(request, receiver, startup, status, &cancellation);
                })
                .map_err(|_| TerminalTapeRecordingError::WorkerUnavailable)?;
            // Sender remains owned by this future while setup is pending.
            // Dropping the future closes admission even before startup replies.
            let path = response.await?;
            Ok(Self {
                sender: Some(sender),
                path,
                record_stdin,
                completion,
            })
        })
    }

    /// Returns the caller-authorized recording path, never an error diagnostic.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns metadata only; retaining it cannot retain admission or file I/O.
    #[must_use]
    pub fn completion(&self) -> TerminalTapeRecordingCompletion {
        self.completion.clone()
    }

    /// Nonblocking bounded admission on first poll, followed by an async receipt.
    /// `Busy` means nothing was admitted: keep the accepted terminal prefix and
    /// retry. Once admitted, dropping this future does not cancel the write.
    pub fn record<'a>(
        &'a mut self,
        frame: TerminalTapeRecordingFrame<'a>,
        timestamp_ms: i64,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<TerminalTapeRecordingStatus, TerminalTapeRecordingError>> {
        Box::pin(async move {
            check_cancelled(&cancellation)?;
            let current = self.completion.status();
            if !current.active {
                return Err(current
                    .failure
                    .unwrap_or(TerminalTapeRecordingError::Closed));
            }
            let payload = match encode_frame(frame, self.record_stdin).transpose() {
                Ok(payload) => payload,
                Err(error) => {
                    let mut status = self
                        .completion
                        .status
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    status.active = false;
                    status.failure.get_or_insert(error);
                    drop(status);
                    self.sender.take();
                    return Err(error);
                }
            };
            let Some(payload) = payload else {
                return Ok(current);
            };
            let (reply, receipt) = reply::channel();
            self.sender
                .as_ref()
                .ok_or(TerminalTapeRecordingError::Closed)?
                .try_send(Command::Frame {
                    payload,
                    timestamp_ms,
                    cancellation,
                    reply,
                })
                .map_err(map_send_error)?;
            receipt.await
        })
    }

    /// Requests flush/sync/close after admitted frames. Call before closing the
    /// host scope, then use its completion fence to prove actual thread join.
    /// Dropping this future or the recorder cannot detach finalization.
    pub fn finish(
        &mut self,
    ) -> BoxFuture<'_, Result<TerminalTapeRecordingStatus, TerminalTapeRecordingError>> {
        Box::pin(async move {
            let current = self.completion.status();
            if current.closed {
                self.sender.take();
                return Ok(current);
            }
            let (reply, receipt) = reply::channel();
            self.sender
                .as_ref()
                .ok_or(TerminalTapeRecordingError::Closed)?
                .try_send(Command::Finish(reply))
                .map_err(map_send_error)?;
            self.sender.take();
            receipt.await
        })
    }
}

fn map_send_error<T>(error: mpsc::TrySendError<T>) -> TerminalTapeRecordingError {
    match error {
        mpsc::TrySendError::Full(_) => TerminalTapeRecordingError::Busy,
        mpsc::TrySendError::Disconnected(_) => TerminalTapeRecordingError::Closed,
    }
}

struct FramePayload {
    kind: u8,
    bytes: Vec<u8>,
}

fn encode_frame(
    frame: TerminalTapeRecordingFrame<'_>,
    record_stdin: bool,
) -> Option<Result<FramePayload, TerminalTapeRecordingError>> {
    let resize;
    let (kind, bytes) = match frame {
        TerminalTapeRecordingFrame::StdoutWritten([])
        | TerminalTapeRecordingFrame::StdinAccepted([]) => return None,
        TerminalTapeRecordingFrame::StdinAccepted(_) if !record_stdin => return None,
        TerminalTapeRecordingFrame::StdoutWritten(bytes) => (1, bytes),
        TerminalTapeRecordingFrame::StdinAccepted(bytes) => (2, bytes),
        TerminalTapeRecordingFrame::Resize { cols, rows } => {
            if !valid_dimensions(cols, rows) {
                return Some(Err(TerminalTapeRecordingError::InvalidRequest));
            }
            resize = [cols.to_le_bytes(), rows.to_le_bytes()].concat();
            (3, resize.as_slice())
        }
        TerminalTapeRecordingFrame::Sigint => (4, &[][..]),
        TerminalTapeRecordingFrame::Marker(bytes) => (5, bytes),
    };
    if bytes.len() > MAX_TERMINAL_TAPE_RECORDING_FRAME_BYTES {
        return Some(Err(TerminalTapeRecordingError::LimitExceeded));
    }
    Some(Ok(FramePayload {
        kind,
        bytes: bytes.to_vec(),
    }))
}

enum Command {
    Frame {
        payload: FramePayload,
        timestamp_ms: i64,
        cancellation: CancellationToken,
        reply: reply::Sender<TerminalTapeRecordingStatus>,
    },
    Finish(reply::Sender<TerminalTapeRecordingStatus>),
}

trait TapeSink: Write + Send {
    fn sync(&mut self) -> io::Result<()>;
}

impl TapeSink for std::fs::File {
    fn sync(&mut self) -> io::Result<()> {
        self.sync_all()
    }
}

struct TapeWriter {
    sink: Option<Box<dyn TapeSink>>,
    status: Arc<Mutex<TerminalTapeRecordingStatus>>,
    options: TerminalTapeRecordingOptions,
    last_ms: i64,
}

impl TapeWriter {
    fn update(&self, apply: impl FnOnce(&mut TerminalTapeRecordingStatus)) {
        apply(
            &mut self
                .status
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
    }

    fn snapshot(&self) -> TerminalTapeRecordingStatus {
        *self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write_all(
        &mut self,
        bytes: &[u8],
        cancellation: &CancellationToken,
    ) -> Result<(), TerminalTapeRecordingError> {
        let mut offset = 0;
        for _ in 0..MAX_WRITE_ATTEMPTS {
            check_cancelled(cancellation)?;
            if offset == bytes.len() {
                return Ok(());
            }
            let result = self
                .sink
                .as_mut()
                .ok_or(TerminalTapeRecordingError::Closed)?
                .write(&bytes[offset..]);
            match result {
                Ok(0) => return Err(TerminalTapeRecordingError::WriteFailed),
                Ok(count) if count <= bytes.len() - offset => {
                    offset += count;
                    self.update(|status| status.bytes_written += count as u64);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Ok(_) | Err(_) => return Err(TerminalTapeRecordingError::WriteFailed),
            }
            check_cancelled(cancellation)?;
        }
        if offset == bytes.len() {
            Ok(())
        } else {
            Err(TerminalTapeRecordingError::LimitExceeded)
        }
    }

    fn frame(
        &mut self,
        payload: &FramePayload,
        timestamp_ms: i64,
        cancellation: &CancellationToken,
    ) -> Result<TerminalTapeRecordingStatus, TerminalTapeRecordingError> {
        let current = self.snapshot();
        let bytes = 9 + payload.bytes.len() as u64;
        if current.frames >= self.options.max_frames
            || bytes > self.options.max_bytes - current.bytes_written
        {
            return Err(TerminalTapeRecordingError::LimitExceeded);
        }
        let delta = i32::try_from(
            timestamp_ms
                .saturating_sub(self.last_ms)
                .clamp(0, i64::from(i32::MAX)),
        )
        .expect("delta clamped");
        let mut header = [0; 9];
        header[..4].copy_from_slice(&delta.to_le_bytes());
        header[4] = payload.kind;
        header[5..].copy_from_slice(
            &u32::try_from(payload.bytes.len())
                .expect("bounded payload")
                .to_le_bytes(),
        );
        let result = self
            .write_all(&header, cancellation)
            .and_then(|()| self.write_all(&payload.bytes, cancellation));
        // A post-write cancellation may follow an entirely committed frame.
        // Keep its actual byte/frame receipt even though recording then fails.
        if self.snapshot().bytes_written == current.bytes_written + bytes {
            self.last_ms = timestamp_ms;
            self.update(|status| status.frames += 1);
        }
        result?;
        Ok(self.snapshot())
    }

    fn close(
        &mut self,
        failure: Option<TerminalTapeRecordingError>,
    ) -> TerminalTapeRecordingStatus {
        self.update(|status| {
            status.active = false;
            status.failure = status.failure.or(failure);
        });
        if let Some(mut sink) = self.sink.take() {
            if sink.flush().and_then(|()| sink.sync()).is_err() {
                self.update(|status| {
                    status
                        .failure
                        .get_or_insert(TerminalTapeRecordingError::FlushFailed);
                });
            }
            drop(sink);
        }
        self.update(|status| {
            status.closed = true;
            status.complete = status.failure.is_none();
        });
        self.snapshot()
    }
}

impl Drop for TapeWriter {
    fn drop(&mut self) {
        if self.sink.is_some() {
            self.close(Some(TerminalTapeRecordingError::Abandoned));
        }
    }
}

fn run_recording(
    request: TerminalTapeRecordingRequest,
    receiver: mpsc::Receiver<Command>,
    startup: reply::Sender<PathBuf>,
    status: Arc<Mutex<TerminalTapeRecordingStatus>>,
    cancellation: &CancellationToken,
) {
    let opened = filesystem::open(request.destination, request.options.epoch_ms, cancellation);
    let (file, path) = match opened {
        Ok(opened) => opened,
        Err(error) => {
            let mut status = status
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            status.closed = true;
            status.failure = Some(error);
            drop(status);
            startup.send(Err(error));
            return;
        }
    };
    let mut writer = TapeWriter {
        sink: Some(Box::new(file)),
        last_ms: request.options.epoch_ms,
        options: request.options,
        status,
    };
    let mut header = Vec::with_capacity(18 + writer.options.version.len());
    header.extend_from_slice(b"FXTP\x01");
    header.extend_from_slice(&writer.options.cols.to_le_bytes());
    header.extend_from_slice(&writer.options.rows.to_le_bytes());
    header.extend_from_slice(&writer.options.epoch_ms.to_le_bytes());
    header.push(u8::try_from(writer.options.version.len()).expect("validated version"));
    header.extend_from_slice(&writer.options.version);
    if let Err(error) = writer.write_all(&header, cancellation) {
        writer.close(Some(error));
        startup.send(Err(error));
        return;
    }
    writer.update(|status| status.active = true);
    startup.send(Ok(path));
    serve(writer, receiver);
}

fn serve(mut writer: TapeWriter, receiver: mpsc::Receiver<Command>) {
    while let Ok(command) = receiver.recv() {
        match command {
            Command::Frame {
                payload,
                timestamp_ms,
                cancellation,
                reply,
            } => {
                let result = writer.frame(&payload, timestamp_ms, &cancellation);
                if let Err(error) = result {
                    let status = writer.close(Some(error));
                    reply.send(Err(error));
                    // Preserve the initiating failure for an already queued
                    // command, rather than relabeling it as a lost worker.
                    for pending in receiver.try_iter() {
                        match pending {
                            Command::Frame { reply, .. } => reply.send(Err(error)),
                            Command::Finish(reply) => reply.send(Ok(status)),
                        }
                    }
                    return;
                }
                reply.send(result);
            }
            Command::Finish(reply) => {
                reply.send(Ok(writer.close(None)));
                return;
            }
        }
    }
    writer.close(Some(TerminalTapeRecordingError::Abandoned));
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<(), TerminalTapeRecordingError> {
    if cancellation.is_cancelled() {
        Err(TerminalTapeRecordingError::Cancelled)
    } else {
        Ok(())
    }
}

fn valid_dimensions(cols: u16, rows: u16) -> bool {
    cols > 0
        && rows > 0
        && cols <= 4096
        && rows <= 4096
        && u32::from(cols) * u32::from(rows) <= 262_144
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests;
