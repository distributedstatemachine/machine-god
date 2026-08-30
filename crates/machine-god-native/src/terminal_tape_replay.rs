//! Bounded replay of explicit FXTP v1 terminal-tape paths.

use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use machine_god_core::{BoxFuture, CancellationToken};
use memchr::memmem;

#[cfg(unix)]
use rustix::fs::{Mode, OFlags};

use crate::terminal_grid::{TerminalGrid, TerminalGridError, TerminalGridFeedError};

/// Exclusive upper bound for an input FXTP v1 tape.
pub const MAX_TERMINAL_TAPE_BYTES: usize = 64 * 1024 * 1024;
/// Inclusive upper bound for bytes returned on stdout and stderr.
pub const MAX_TERMINAL_TAPE_RENDERED_OUTPUT_BYTES: usize = 128 * 1024 * 1024;
/// Inclusive upper bound for files written beneath `--frames-dir`.
pub const MAX_TERMINAL_TAPE_ARTIFACT_BYTES: usize = 128 * 1024 * 1024;
/// Maximum complete frames materialized beneath `--frames-dir`.
pub const MAX_TERMINAL_TAPE_FRAMES_DIR_FRAMES: usize = 4096;

const MAGIC: &[u8; 5] = b"FXTP\x01";
const FIXED_HEADER_BYTES: usize = MAGIC.len() + 2 + 2 + 8 + 1;
const FRAME_HEADER_BYTES: usize = 9;
const IO_CHUNK_BYTES: usize = 16 * 1024;
const INCOMPLETE_WARNING: &[u8] = b"machine-god replay: ignored incomplete final tape frame\n";

/// Explicit authority and output options for one terminal-tape replay.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalTapeReplayRequest {
    tape: PathBuf,
    frames: bool,
    json: bool,
    golden: Option<PathBuf>,
    frames_dir: Option<PathBuf>,
}

impl TerminalTapeReplayRequest {
    /// Creates one request. Paths are explicit caller-granted filesystem authority.
    #[must_use]
    pub const fn new(
        tape: PathBuf,
        frames: bool,
        json: bool,
        golden: Option<PathBuf>,
        frames_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            tape,
            frames,
            json,
            golden,
            frames_dir,
        }
    }

    /// Returns the explicit tape path.
    #[must_use]
    pub fn tape(&self) -> &Path {
        &self.tape
    }

    /// Returns whether intermediate non-marker frames are rendered to stdout.
    #[must_use]
    pub const fn frames(&self) -> bool {
        self.frames
    }

    /// Returns whether the FX-compatible JSON summary is rendered to stdout.
    #[must_use]
    pub const fn json(&self) -> bool {
        self.json
    }

    /// Returns the optional explicit final-grid output path.
    #[must_use]
    pub fn golden(&self) -> Option<&Path> {
        self.golden.as_deref()
    }

    /// Returns the optional explicit per-frame artifact root.
    #[must_use]
    pub fn frames_dir(&self) -> Option<&Path> {
        self.frames_dir.as_deref()
    }
}

impl fmt::Debug for TerminalTapeReplayRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalTapeReplayRequest")
            .field("has_tape", &true)
            .field("frames", &self.frames)
            .field("json", &self.json)
            .field("has_golden", &self.golden.is_some())
            .field("has_frames_dir", &self.frames_dir.is_some())
            .finish_non_exhaustive()
    }
}

/// Captured successful replay output.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalTapeReplayOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl TerminalTapeReplayOutput {
    /// Constructs a bounded captured output, primarily for injected CLI hosts.
    ///
    /// # Errors
    ///
    /// Returns `ResourceLimit` when the combined buffers exceed the replay cap.
    pub fn from_parts(stdout: Vec<u8>, stderr: Vec<u8>) -> Result<Self, TerminalTapeReplayError> {
        let total = stdout
            .len()
            .checked_add(stderr.len())
            .ok_or_else(resource_limit)?;
        if total > MAX_TERMINAL_TAPE_RENDERED_OUTPUT_BYTES {
            return Err(resource_limit());
        }
        Ok(Self { stdout, stderr })
    }

    /// Returns bytes destined for stdout.
    #[must_use]
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    /// Returns bytes destined for stderr.
    #[must_use]
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    /// Consumes the output into `(stdout, stderr)`.
    #[must_use]
    pub fn into_parts(self) -> (Vec<u8>, Vec<u8>) {
        (self.stdout, self.stderr)
    }
}

impl fmt::Debug for TerminalTapeReplayOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalTapeReplayOutput")
            .field("stdout_bytes", &self.stdout.len())
            .field("stderr_bytes", &self.stderr.len())
            .finish()
    }
}

/// Stable terminal-tape replay failure category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TerminalTapeReplayErrorKind {
    /// The explicit tape path did not exist.
    FileNotFound,
    /// The explicit tape could not be opened.
    OpenFailed,
    /// The opened tape could not be read completely.
    ReadFailed,
    /// The FXTP header, grid dimensions, or terminal stream was invalid.
    BadTape,
    /// The explicit golden path could not be written.
    WriteFailed,
    /// The explicit frames directory or an artifact could not be written.
    FramesDirFailed,
    /// A checked input, output, frame, artifact, counter, or grid limit was exceeded.
    ResourceLimit,
    /// Cooperative cancellation was requested.
    Cancelled,
}

impl TerminalTapeReplayErrorKind {
    /// Returns the stable FX-compatible machine-readable code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::FileNotFound => "FileNotFound",
            Self::OpenFailed => "OpenFailed",
            Self::ReadFailed => "ReadFailed",
            Self::BadTape => "BadTape",
            Self::WriteFailed => "WriteFailed",
            Self::FramesDirFailed => "FramesDirFailed",
            Self::ResourceLimit => "ResourceLimit",
            Self::Cancelled => "Cancelled",
        }
    }

    const fn message(self) -> &'static str {
        match self {
            Self::FileNotFound => "cannot open tape: FileNotFound",
            Self::OpenFailed => "cannot open tape: OpenFailed",
            Self::ReadFailed => "read failed: ReadFailed",
            Self::BadTape => "bad tape: BadTape",
            Self::WriteFailed => "write failed: WriteFailed",
            Self::FramesDirFailed => "cannot prepare frames directory: FramesDirFailed",
            Self::ResourceLimit => "resource limit exceeded: ResourceLimit",
            Self::Cancelled => "replay cancelled: Cancelled",
        }
    }
}

/// Fixed, path-free failure to replay one explicit terminal tape.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct TerminalTapeReplayError {
    kind: TerminalTapeReplayErrorKind,
}

impl TerminalTapeReplayError {
    /// Returns the stable failure category.
    #[must_use]
    pub const fn kind(self) -> TerminalTapeReplayErrorKind {
        self.kind
    }

    /// Returns the stable machine-readable code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        self.kind.code()
    }

    /// Returns the stable redacted human message without a trailing newline.
    #[must_use]
    pub const fn message(self) -> &'static str {
        self.kind.message()
    }

    const fn new(kind: TerminalTapeReplayErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for TerminalTapeReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalTapeReplayError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for TerminalTapeReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl Error for TerminalTapeReplayError {}

/// Replays one FXTP v1 tape through the bounded native terminal grid.
///
/// Construction is effect-inert. The first poll checks cancellation before
/// opening the caller-authorized tape. This function does not inspect process
/// configuration, state, credentials, sessions, providers, or the network.
/// Filesystem work may block the polling thread.
#[must_use]
pub fn replay_terminal_tape(
    request: TerminalTapeReplayRequest,
    cancellation: CancellationToken,
) -> BoxFuture<'static, Result<TerminalTapeReplayOutput, TerminalTapeReplayError>> {
    Box::pin(async move { replay_polled(&request, &cancellation) })
}

#[derive(Clone, Copy)]
struct Header<'a> {
    cols: u16,
    rows: u16,
    epoch_ms: i64,
    version: &'a [u8],
}

#[derive(Clone, Copy)]
struct Frame<'a> {
    delta_ms: i32,
    kind: u8,
    payload: &'a [u8],
}

struct CapturedOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

struct VisibleMarkersCache {
    encoded: Vec<u8>,
    evaluated: usize,
    has_visible: bool,
}

impl VisibleMarkersCache {
    fn new() -> Self {
        Self {
            encoded: Vec::new(),
            evaluated: 0,
            has_visible: false,
        }
    }

    fn update(
        &mut self,
        snapshot: &[u8],
        markers: &[&[u8]],
        grid_changed: bool,
    ) -> Result<(), TerminalTapeReplayError> {
        if grid_changed {
            self.encoded.clear();
            self.evaluated = 0;
            self.has_visible = false;
        }
        for marker in &markers[self.evaluated..] {
            if marker.is_empty() || !contains_bytes(snapshot, marker) {
                continue;
            }
            if self.has_visible {
                push_internal(&mut self.encoded, b",")?;
            }
            push_json_bytes(&mut self.encoded, marker)?;
            self.has_visible = true;
        }
        self.evaluated = markers.len();
        Ok(())
    }

    fn encoded(&self) -> &[u8] {
        &self.encoded
    }
}

impl CapturedOutput {
    fn new() -> Self {
        Self {
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), TerminalTapeReplayError> {
        append_bounded_output(&mut self.stdout, self.stderr.len(), bytes)
    }

    fn write_stderr(&mut self, bytes: &[u8]) -> Result<(), TerminalTapeReplayError> {
        append_bounded_output(&mut self.stderr, self.stdout.len(), bytes)
    }

    fn finish(self) -> Result<TerminalTapeReplayOutput, TerminalTapeReplayError> {
        TerminalTapeReplayOutput::from_parts(self.stdout, self.stderr)
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    position: usize,
    header: Header<'a>,
}

impl<'a> Parser<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self, TerminalTapeReplayError> {
        if bytes.len() < FIXED_HEADER_BYTES || &bytes[..MAGIC.len()] != MAGIC {
            return Err(bad_tape());
        }
        let mut position = MAGIC.len();
        let cols = read_u16(bytes, position)?;
        position += 2;
        let rows = read_u16(bytes, position)?;
        position += 2;
        let epoch_ms = read_i64(bytes, position)?;
        position += 8;
        let version_len = usize::from(bytes[position]);
        position += 1;
        let version_end = position.checked_add(version_len).ok_or_else(bad_tape)?;
        let version = bytes.get(position..version_end).ok_or_else(bad_tape)?;
        Ok(Self {
            bytes,
            position: version_end,
            header: Header {
                cols,
                rows,
                epoch_ms,
                version,
            },
        })
    }

    fn next(&mut self) -> Result<Option<Frame<'a>>, IncompleteFrame> {
        if self.position >= self.bytes.len() {
            return Ok(None);
        }
        let Some(header_end) = self.position.checked_add(FRAME_HEADER_BYTES) else {
            return Err(IncompleteFrame);
        };
        if header_end > self.bytes.len() {
            return Err(IncompleteFrame);
        }
        let delta_ms = i32::from_le_bytes(
            self.bytes[self.position..self.position + 4]
                .try_into()
                .expect("checked frame header"),
        );
        let kind = self.bytes[self.position + 4];
        let payload_len = usize::try_from(u32::from_le_bytes(
            self.bytes[self.position + 5..header_end]
                .try_into()
                .expect("checked frame header"),
        ))
        .map_err(|_| IncompleteFrame)?;
        let Some(payload_end) = header_end.checked_add(payload_len) else {
            return Err(IncompleteFrame);
        };
        let Some(payload) = self.bytes.get(header_end..payload_end) else {
            return Err(IncompleteFrame);
        };
        self.position = payload_end;
        Ok(Some(Frame {
            delta_ms,
            kind,
            payload,
        }))
    }
}

#[derive(Clone, Copy)]
struct IncompleteFrame;

#[allow(clippy::too_many_lines)] // Preserve the pinned FX replay side-effect order in one audit scope.
fn replay_polled(
    request: &TerminalTapeReplayRequest,
    cancellation: &CancellationToken,
) -> Result<TerminalTapeReplayOutput, TerminalTapeReplayError> {
    replay_polled_with_grid_checkpoint(request, cancellation, || {})
}

#[allow(clippy::too_many_lines)] // Preserve the pinned FX replay side-effect order in one audit scope.
fn replay_polled_with_grid_checkpoint(
    request: &TerminalTapeReplayRequest,
    cancellation: &CancellationToken,
    mut before_grid_checkpoint: impl FnMut(),
) -> Result<TerminalTapeReplayOutput, TerminalTapeReplayError> {
    check_cancelled(cancellation)?;
    let tape = read_tape(&request.tape, cancellation)?;
    let mut parser = Parser::new(&tape)?;
    let header = parser.header;
    let mut grid = TerminalGrid::new(header.cols, header.rows).map_err(map_grid_error)?;
    let mut output = CapturedOutput::new();
    let mut summary = Vec::new();
    if request.json {
        push_internal(&mut summary, b"{\"cols\":")?;
        push_number(&mut summary, u64::from(header.cols))?;
        push_internal(&mut summary, b",\"rows\":")?;
        push_number(&mut summary, u64::from(header.rows))?;
        push_internal(&mut summary, b",\"epoch_ms\":")?;
        push_signed_number(&mut summary, header.epoch_ms)?;
        push_internal(&mut summary, b",\"version\":")?;
        push_json_bytes(&mut summary, header.version)?;
        push_internal(&mut summary, b",\"frames\":[")?;
    }

    let mut artifact_bytes = 0usize;
    if let Some(root) = request.frames_dir.as_deref() {
        check_cancelled(cancellation)?;
        prepare_frames_dir(root).map_err(|_| frames_dir_failed())?;
    }

    let mut frame_count = 0usize;
    let mut resize_count = 0usize;
    let mut stdout_bytes = 0usize;
    let mut elapsed_ms = 0i64;
    let mut markers: Vec<&[u8]> = Vec::new();
    let mut visible_markers = VisibleMarkersCache::new();
    let mut first_json_frame = true;
    let mut ignored_incomplete_tail = false;

    loop {
        check_cancelled(cancellation)?;
        let frame = match parser.next() {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(IncompleteFrame) => {
                ignored_incomplete_tail = true;
                break;
            }
        };
        frame_count = frame_count.checked_add(1).ok_or_else(resource_limit)?;
        if request.frames_dir.is_some() && frame_count > MAX_TERMINAL_TAPE_FRAMES_DIR_FRAMES {
            return Err(resource_limit());
        }
        elapsed_ms = elapsed_ms
            .checked_add(i64::from(frame.delta_ms))
            .ok_or_else(resource_limit)?;

        let mut grid_changed = false;
        match frame.kind {
            1 => {
                stdout_bytes = stdout_bytes
                    .checked_add(frame.payload.len())
                    .ok_or_else(resource_limit)?;
                check_cancelled(cancellation)?;
                grid.feed_with_cancel_check(frame.payload, || {
                    before_grid_checkpoint();
                    cancellation.is_cancelled()
                })
                .map_err(map_grid_feed_error)?;
                check_cancelled(cancellation)?;
                grid_changed = true;
            }
            3 if frame.payload.len() >= 4 => {
                let cols = u16::from_le_bytes([frame.payload[0], frame.payload[1]]);
                let rows = u16::from_le_bytes([frame.payload[2], frame.payload[3]]);
                grid.resize(cols, rows).map_err(map_grid_error)?;
                resize_count = resize_count.checked_add(1).ok_or_else(resource_limit)?;
                grid_changed = true;
            }
            5 if request.frames_dir.is_some() => markers.push(frame.payload),
            _ => {}
        }

        if request.json {
            if !first_json_frame {
                push_internal(&mut summary, b",")?;
            }
            first_json_frame = false;
            push_internal(&mut summary, b"{\"delta_ms\":")?;
            push_signed_number(&mut summary, i64::from(frame.delta_ms))?;
            push_internal(&mut summary, b",\"kind\":")?;
            push_json_bytes(&mut summary, frame_kind_name(frame.kind).as_bytes())?;
            push_internal(&mut summary, b",\"len\":")?;
            push_usize(&mut summary, frame.payload.len())?;
            push_internal(&mut summary, b"}")?;
        }

        let snapshot = if request.frames || request.frames_dir.is_some() {
            Some(grid.snapshot().map_err(map_grid_error)?)
        } else {
            None
        };

        if request.frames && frame.kind != 5 {
            let heading = format!(
                "\n--- frame {frame_count} ({}, +{}ms) ---\n",
                frame_kind_name(frame.kind),
                frame.delta_ms
            );
            output.write_stdout(heading.as_bytes())?;
            output.write_stdout(snapshot.as_deref().expect("snapshot requested for frames"))?;
        }

        if let Some(root) = request.frames_dir.as_deref() {
            let snapshot = snapshot
                .as_deref()
                .expect("snapshot requested for frames directory");
            visible_markers.update(snapshot, &markers, grid_changed)?;
            let metadata = frame_metadata(
                frame_count,
                frame,
                elapsed_ms,
                &grid,
                snapshot,
                visible_markers.encoded(),
            )?;
            write_artifact(
                &root.join("frames").join(format!("{frame_count:04}.json")),
                &metadata,
                &mut artifact_bytes,
                cancellation,
            )?;
            write_artifact(
                &root
                    .join("frames")
                    .join(format!("{frame_count:04}.grid.txt")),
                snapshot,
                &mut artifact_bytes,
                cancellation,
            )?;
        }
    }

    if request.json {
        push_internal(&mut summary, b"],\"frame_count\":")?;
        push_usize(&mut summary, frame_count)?;
        push_internal(&mut summary, b",\"resize_count\":")?;
        push_usize(&mut summary, resize_count)?;
        push_internal(&mut summary, b",\"stdout_bytes\":")?;
        push_usize(&mut summary, stdout_bytes)?;
        push_internal(&mut summary, b"}\n")?;
        output.write_stdout(&summary)?;
    }
    if ignored_incomplete_tail {
        output.write_stderr(INCOMPLETE_WARNING)?;
    }

    if let Some(root) = request.frames_dir.as_deref() {
        let manifest = frames_manifest(header, frame_count, resize_count, stdout_bytes)?;
        write_artifact(
            &root.join("manifest.json"),
            &manifest,
            &mut artifact_bytes,
            cancellation,
        )?;
    }

    check_cancelled(cancellation)?;
    let final_snapshot = grid.snapshot().map_err(map_grid_error)?;
    if final_snapshot.len() > MAX_TERMINAL_TAPE_RENDERED_OUTPUT_BYTES {
        return Err(resource_limit());
    }
    if let Some(path) = request.golden.as_deref() {
        write_file(path, &final_snapshot, cancellation)
            .map_err(|error| map_golden_write_error(&error, cancellation))?;
        return output.finish();
    }
    if !request.frames && !request.json {
        output.write_stdout(&final_snapshot)?;
    }
    output.finish()
}

fn map_grid_feed_error(error: TerminalGridFeedError) -> TerminalTapeReplayError {
    match error {
        TerminalGridFeedError::Grid(error) => map_grid_error(error),
        TerminalGridFeedError::Cancelled => cancelled(),
    }
}

fn read_tape(
    path: &Path,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, TerminalTapeReplayError> {
    check_cancelled(cancellation)?;
    let mut file = open_regular_tape(path, cancellation)?;
    let mut bytes = Vec::new();
    let mut buffer = [0u8; IO_CHUNK_BYTES];
    loop {
        check_cancelled(cancellation)?;
        let Ok(read) = file.read(&mut buffer) else {
            check_cancelled(cancellation)?;
            return Err(read_failed());
        };
        check_cancelled(cancellation)?;
        if read == 0 {
            break;
        }
        let next = bytes.len().checked_add(read).ok_or_else(resource_limit)?;
        if next >= MAX_TERMINAL_TAPE_BYTES {
            return Err(resource_limit());
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    Ok(bytes)
}

fn prepare_frames_dir(root: &Path) -> io::Result<()> {
    fs::create_dir_all(root)?;
    fs::create_dir_all(root.join("frames"))
}

fn write_artifact(
    path: &Path,
    bytes: &[u8],
    total: &mut usize,
    cancellation: &CancellationToken,
) -> Result<(), TerminalTapeReplayError> {
    let next = total.checked_add(bytes.len()).ok_or_else(resource_limit)?;
    if next > MAX_TERMINAL_TAPE_ARTIFACT_BYTES {
        return Err(resource_limit());
    }
    write_file(path, bytes, cancellation).map_err(|error| {
        if error.kind() == io::ErrorKind::Interrupted && cancellation.is_cancelled() {
            cancelled()
        } else {
            frames_dir_failed()
        }
    })?;
    *total = next;
    Ok(())
}

fn write_file(path: &Path, bytes: &[u8], cancellation: &CancellationToken) -> io::Result<()> {
    if cancellation.is_cancelled() {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
    }
    let mut file = open_regular_output(path, cancellation)?;
    for chunk in bytes.chunks(IO_CHUNK_BYTES) {
        if cancellation.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        file.write_all(chunk)?;
        if cancellation.is_cancelled() {
            return Err(cancelled_io_error());
        }
    }
    Ok(())
}

#[cfg(unix)]
fn open_regular_tape(
    path: &Path,
    cancellation: &CancellationToken,
) -> Result<File, TerminalTapeReplayError> {
    let descriptor = match rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(descriptor) => descriptor,
        Err(error) => {
            check_cancelled(cancellation)?;
            return Err(if error == rustix::io::Errno::NOENT {
                TerminalTapeReplayError::new(TerminalTapeReplayErrorKind::FileNotFound)
            } else {
                TerminalTapeReplayError::new(TerminalTapeReplayErrorKind::OpenFailed)
            });
        }
    };
    check_cancelled(cancellation)?;
    let file = File::from(descriptor);
    let metadata = file
        .metadata()
        .map_err(|_| TerminalTapeReplayError::new(TerminalTapeReplayErrorKind::OpenFailed))?;
    if !metadata.is_file() {
        return Err(TerminalTapeReplayError::new(
            TerminalTapeReplayErrorKind::OpenFailed,
        ));
    }
    Ok(file)
}

#[cfg(not(unix))]
fn open_regular_tape(
    path: &Path,
    cancellation: &CancellationToken,
) -> Result<File, TerminalTapeReplayError> {
    match fs::metadata(path) {
        Ok(metadata) if !metadata.is_file() => {
            return Err(TerminalTapeReplayError::new(
                TerminalTapeReplayErrorKind::OpenFailed,
            ));
        }
        Ok(_) => {}
        Err(error) => {
            check_cancelled(cancellation)?;
            return Err(map_open_error(&error));
        }
    }
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) => {
            check_cancelled(cancellation)?;
            return Err(map_open_error(&error));
        }
    };
    check_cancelled(cancellation)?;
    if !file
        .metadata()
        .map_err(|_| TerminalTapeReplayError::new(TerminalTapeReplayErrorKind::OpenFailed))?
        .is_file()
    {
        return Err(TerminalTapeReplayError::new(
            TerminalTapeReplayErrorKind::OpenFailed,
        ));
    }
    Ok(file)
}

#[cfg(unix)]
fn open_regular_output(path: &Path, cancellation: &CancellationToken) -> io::Result<File> {
    let descriptor = match rustix::fs::open(
        path,
        OFlags::WRONLY | OFlags::CREATE | OFlags::TRUNC | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::from_raw_mode(0o666),
    ) {
        Ok(descriptor) => descriptor,
        Err(error) => {
            if cancellation.is_cancelled() {
                return Err(cancelled_io_error());
            }
            return Err(error.into());
        }
    };
    if cancellation.is_cancelled() {
        return Err(cancelled_io_error());
    }
    let file = File::from(descriptor);
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("output target is not a regular file"));
    }
    Ok(file)
}

#[cfg(not(unix))]
fn open_regular_output(path: &Path, cancellation: &CancellationToken) -> io::Result<File> {
    match fs::metadata(path) {
        Ok(metadata) if !metadata.is_file() => {
            return Err(io::Error::other("output target is not a regular file"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if cancellation.is_cancelled() {
        return Err(cancelled_io_error());
    }
    let file = File::create(path)?;
    if cancellation.is_cancelled() {
        return Err(cancelled_io_error());
    }
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("output target is not a regular file"));
    }
    Ok(file)
}

fn cancelled_io_error() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "cancelled")
}

fn frame_metadata(
    index: usize,
    frame: Frame<'_>,
    elapsed_ms: i64,
    grid: &TerminalGrid,
    snapshot: &[u8],
    visible_markers: &[u8],
) -> Result<Vec<u8>, TerminalTapeReplayError> {
    let mut out = Vec::new();
    push_internal(&mut out, b"{\"index\":")?;
    push_usize(&mut out, index)?;
    push_internal(&mut out, b",\"delta_ms\":")?;
    push_signed_number(&mut out, i64::from(frame.delta_ms))?;
    push_internal(&mut out, b",\"elapsed_ms\":")?;
    push_signed_number(&mut out, elapsed_ms)?;
    push_internal(&mut out, b",\"kind\":")?;
    push_json_bytes(&mut out, frame_kind_name(frame.kind).as_bytes())?;
    push_internal(&mut out, b",\"payload_len\":")?;
    push_usize(&mut out, frame.payload.len())?;
    push_internal(&mut out, b",\"size\":{\"cols\":")?;
    push_number(&mut out, u64::from(grid.cols()))?;
    push_internal(&mut out, b",\"rows\":")?;
    push_number(&mut out, u64::from(grid.rows()))?;
    push_internal(&mut out, b"},\"cursor\":{\"row\":")?;
    push_number(&mut out, u64::from(grid.cursor_row()))?;
    push_internal(&mut out, b",\"col\":")?;
    push_number(&mut out, u64::from(grid.cursor_col()))?;
    push_internal(&mut out, b",\"visible\":")?;
    push_internal(
        &mut out,
        if grid.cursor_visible() {
            b"true"
        } else {
            b"false"
        },
    )?;
    push_internal(&mut out, b"},\"footer_candidates\":")?;
    push_footer_candidates(&mut out, snapshot)?;
    push_internal(&mut out, b",\"visible_markers\":")?;
    push_internal(&mut out, b"[")?;
    push_internal(&mut out, visible_markers)?;
    push_internal(&mut out, b"]")?;
    push_internal(&mut out, b"}\n")?;
    Ok(out)
}

fn frames_manifest(
    header: Header<'_>,
    frame_count: usize,
    resize_count: usize,
    stdout_bytes: usize,
) -> Result<Vec<u8>, TerminalTapeReplayError> {
    let mut out = Vec::new();
    push_internal(&mut out, b"{\"cols\":")?;
    push_number(&mut out, u64::from(header.cols))?;
    push_internal(&mut out, b",\"rows\":")?;
    push_number(&mut out, u64::from(header.rows))?;
    push_internal(&mut out, b",\"epoch_ms\":")?;
    push_signed_number(&mut out, header.epoch_ms)?;
    push_internal(&mut out, b",\"version\":")?;
    push_json_bytes(&mut out, header.version)?;
    push_internal(&mut out, b",\"frame_count\":")?;
    push_usize(&mut out, frame_count)?;
    push_internal(&mut out, b",\"resize_count\":")?;
    push_usize(&mut out, resize_count)?;
    push_internal(&mut out, b",\"stdout_bytes\":")?;
    push_usize(&mut out, stdout_bytes)?;
    push_internal(&mut out, b",\"frames_dir\":\"frames\"}\n")?;
    Ok(out)
}

fn push_footer_candidates(
    out: &mut Vec<u8>,
    snapshot: &[u8],
) -> Result<(), TerminalTapeReplayError> {
    let lines: Vec<&[u8]> = snapshot
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    push_internal(out, b"[")?;
    let mut first = true;
    for (index, line) in lines.iter().enumerate() {
        if !is_input_snapshot_row(line)
            || index == 0
            || index + 1 >= lines.len()
            || !is_divider_snapshot_row(lines[index - 1])
            || !is_divider_snapshot_row(lines[index + 1])
        {
            continue;
        }
        if !first {
            push_internal(out, b",")?;
        }
        first = false;
        push_internal(out, b"{\"top_divider\":")?;
        push_usize(out, index)?;
        push_internal(out, b",\"input\":")?;
        push_usize(out, index + 1)?;
        push_internal(out, b",\"bottom_divider\":")?;
        push_usize(out, index + 2)?;
        push_internal(out, b"}")?;
    }
    push_internal(out, b"]")
}

fn is_input_snapshot_row(line: &[u8]) -> bool {
    let text = trim_snapshot_frame(line);
    text.starts_with("❯".as_bytes())
        || text.starts_with(b">")
        || (text.starts_with(b"[")
            && (contains_bytes(text, "] ❯".as_bytes()) || contains_bytes(text, b"] >")))
}

fn is_divider_snapshot_row(line: &[u8]) -> bool {
    let text = trim_snapshot_frame(line);
    contains_bytes(text, "──".as_bytes())
        || contains_bytes(text, "━━".as_bytes())
        || contains_bytes(text, "══".as_bytes())
}

fn trim_snapshot_frame(mut line: &[u8]) -> &[u8] {
    if line.len() >= 2 && line[0] == b'|' && line[line.len() - 1] == b'|' {
        line = &line[1..line.len() - 1];
    }
    while line.last() == Some(&b' ') {
        line = &line[..line.len() - 1];
    }
    line
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && memmem::find(haystack, needle).is_some()
}

fn frame_kind_name(kind: u8) -> &'static str {
    match kind {
        1 => "stdout",
        2 => "stdin",
        3 => "resize",
        4 => "sigint",
        5 => "marker",
        _ => "unknown",
    }
}

fn push_json_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), TerminalTapeReplayError> {
    if std::str::from_utf8(bytes).is_err() {
        push_internal(out, b"[")?;
        for (index, byte) in bytes.iter().enumerate() {
            if index != 0 {
                push_internal(out, b",")?;
            }
            push_number(out, u64::from(*byte))?;
        }
        return push_internal(out, b"]");
    }
    push_internal(out, b"\"")?;
    for byte in bytes {
        match *byte {
            b'"' => push_internal(out, b"\\\"")?,
            b'\\' => push_internal(out, b"\\\\")?,
            b'\n' => push_internal(out, b"\\n")?,
            b'\r' => push_internal(out, b"\\r")?,
            b'\t' => push_internal(out, b"\\t")?,
            0x08 => push_internal(out, b"\\b")?,
            0x0c => push_internal(out, b"\\f")?,
            0x00..=0x1f => {
                let escaped = format!("\\u{:04x}", *byte);
                push_internal(out, escaped.as_bytes())?;
            }
            _ => push_internal(out, &[*byte])?,
        }
    }
    push_internal(out, b"\"")
}

fn push_number(out: &mut Vec<u8>, number: u64) -> Result<(), TerminalTapeReplayError> {
    push_internal(out, number.to_string().as_bytes())
}

fn push_usize(out: &mut Vec<u8>, number: usize) -> Result<(), TerminalTapeReplayError> {
    push_number(out, u64::try_from(number).map_err(|_| resource_limit())?)
}

fn push_signed_number(out: &mut Vec<u8>, number: i64) -> Result<(), TerminalTapeReplayError> {
    push_internal(out, number.to_string().as_bytes())
}

fn push_internal(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), TerminalTapeReplayError> {
    let next = out
        .len()
        .checked_add(bytes.len())
        .ok_or_else(resource_limit)?;
    if next > MAX_TERMINAL_TAPE_RENDERED_OUTPUT_BYTES {
        return Err(resource_limit());
    }
    out.extend_from_slice(bytes);
    Ok(())
}

fn append_bounded_output(
    destination: &mut Vec<u8>,
    other_len: usize,
    bytes: &[u8],
) -> Result<(), TerminalTapeReplayError> {
    let total = destination
        .len()
        .checked_add(other_len)
        .and_then(|value| value.checked_add(bytes.len()))
        .ok_or_else(resource_limit)?;
    if total > MAX_TERMINAL_TAPE_RENDERED_OUTPUT_BYTES {
        return Err(resource_limit());
    }
    destination.extend_from_slice(bytes);
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, TerminalTapeReplayError> {
    let value = bytes.get(offset..offset + 2).ok_or_else(bad_tape)?;
    Ok(u16::from_le_bytes(
        value.try_into().expect("checked two-byte value"),
    ))
}

fn read_i64(bytes: &[u8], offset: usize) -> Result<i64, TerminalTapeReplayError> {
    let value = bytes.get(offset..offset + 8).ok_or_else(bad_tape)?;
    Ok(i64::from_le_bytes(
        value.try_into().expect("checked eight-byte value"),
    ))
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<(), TerminalTapeReplayError> {
    if cancellation.is_cancelled() {
        Err(cancelled())
    } else {
        Ok(())
    }
}

#[cfg(not(unix))]
fn map_open_error(error: &io::Error) -> TerminalTapeReplayError {
    if error.kind() == io::ErrorKind::NotFound {
        TerminalTapeReplayError::new(TerminalTapeReplayErrorKind::FileNotFound)
    } else {
        TerminalTapeReplayError::new(TerminalTapeReplayErrorKind::OpenFailed)
    }
}

fn map_golden_write_error(
    error: &io::Error,
    cancellation: &CancellationToken,
) -> TerminalTapeReplayError {
    if error.kind() == io::ErrorKind::Interrupted && cancellation.is_cancelled() {
        cancelled()
    } else {
        TerminalTapeReplayError::new(TerminalTapeReplayErrorKind::WriteFailed)
    }
}

fn map_grid_error(error: TerminalGridError) -> TerminalTapeReplayError {
    match error {
        TerminalGridError::InvalidGridSize => bad_tape(),
        TerminalGridError::GridTooLarge
        | TerminalGridError::TooManyCsiParameters
        | TerminalGridError::TooManyCsiIntermediates
        | TerminalGridError::ControlStringTooLarge
        | TerminalGridError::SynchronizedUpdateTooLarge
        | TerminalGridError::CombiningPoolCapacityExceeded
        | TerminalGridError::SnapshotTooLarge => resource_limit(),
    }
}

const fn bad_tape() -> TerminalTapeReplayError {
    TerminalTapeReplayError::new(TerminalTapeReplayErrorKind::BadTape)
}

const fn resource_limit() -> TerminalTapeReplayError {
    TerminalTapeReplayError::new(TerminalTapeReplayErrorKind::ResourceLimit)
}

const fn cancelled() -> TerminalTapeReplayError {
    TerminalTapeReplayError::new(TerminalTapeReplayErrorKind::Cancelled)
}

const fn read_failed() -> TerminalTapeReplayError {
    TerminalTapeReplayError::new(TerminalTapeReplayErrorKind::ReadFailed)
}

const fn frames_dir_failed() -> TerminalTapeReplayError {
    TerminalTapeReplayError::new(TerminalTapeReplayErrorKind::FramesDirFailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_PATH: AtomicU64 = AtomicU64::new(0);

    struct TestTape(PathBuf);

    impl TestTape {
        fn with_stdout_payload(payload: &[u8]) -> Self {
            let identifier = NEXT_TEST_PATH.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "machine-god-replay-mid-frame-cancel-{}-{identifier}.fxtape",
                std::process::id()
            ));
            let mut tape = Vec::new();
            tape.extend_from_slice(MAGIC);
            tape.extend_from_slice(&80_u16.to_le_bytes());
            tape.extend_from_slice(&24_u16.to_le_bytes());
            tape.extend_from_slice(&0_i64.to_le_bytes());
            tape.push(0);
            tape.extend_from_slice(&0_i32.to_le_bytes());
            tape.push(1);
            tape.extend_from_slice(
                &u32::try_from(payload.len())
                    .expect("test payload fits the FXTP frame length")
                    .to_le_bytes(),
            );
            tape.extend_from_slice(payload);
            fs::write(&path, tape).expect("write test tape");
            Self(path)
        }
    }

    impl Drop for TestTape {
        fn drop(&mut self) {
            match fs::remove_file(&self.0) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) if std::thread::panicking() => {
                    eprintln!("failed to remove test tape: {error}");
                }
                Err(error) => panic!("failed to remove test tape: {error}"),
            }
        }
    }

    #[test]
    fn cancellation_during_one_complete_stdout_frame_is_distinct() {
        let tape = TestTape::with_stdout_payload(&vec![b'x'; IO_CHUNK_BYTES * 2]);
        let request = TerminalTapeReplayRequest::new(tape.0.clone(), false, false, None, None);
        let cancellation = CancellationToken::new();
        let mut checkpoint_count = 0;

        let error = replay_polled_with_grid_checkpoint(&request, &cancellation, || {
            checkpoint_count += 1;
            if checkpoint_count == 1 {
                assert!(cancellation.cancel());
            }
        })
        .expect_err("mid-frame cancellation fails replay");

        assert_eq!(checkpoint_count, 1);
        assert_eq!(error.kind(), TerminalTapeReplayErrorKind::Cancelled);
    }
}
