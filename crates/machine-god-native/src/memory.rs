use std::error::Error;
use std::fmt;
use std::io;
use std::path::Path;

use machine_god_core::{
    BoxFuture, CancellationToken, Capability, PreparedToolCall, Tool, ToolCall, ToolContext,
    ToolError, ToolErrorKind, ToolName, ToolOutput, ToolSpec,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Value, json};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fs::{AtFlags, FileType, FlockOperation, Mode, OFlags};

/// Version of the durable memory document written by [`MemoryTool`].
pub const MEMORY_SCHEMA_VERSION: u32 = 1;
/// Registered name of [`MemoryTool`].
pub const MEMORY_TOOL_NAME: &str = "memory";
/// Maximum UTF-8 bytes in one memory fact.
pub const MAX_MEMORY_FACT_BYTES: usize = 4_096;
/// Maximum facts in the durable memory set.
pub const MAX_MEMORY_FACTS: usize = 128;
/// Maximum aggregate UTF-8 fact bytes in the durable memory set.
pub const MAX_MEMORY_TOTAL_FACT_BYTES: usize = 32_768;
/// Maximum bytes accepted from or written to `memories.json`.
pub const MAX_MEMORY_FILE_BYTES: usize = 49_152;
/// Maximum serialized bytes in accepted canonical tool arguments.
pub const MAX_MEMORY_SERIALIZED_ARGUMENT_BYTES: usize = 32_768;
/// Maximum serialized bytes in a complete [`ToolOutput`].
pub const MAX_MEMORY_SERIALIZED_RESULT_BYTES: usize = 65_536;
/// Maximum charged native I/O dispatches in one operation.
pub const MAX_MEMORY_IO_ATTEMPTS: usize = 65_536;

#[cfg(any(target_os = "linux", target_os = "macos"))]
const DATA_NAME: &str = "memories.json";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const LOCK_NAME: &str = "memories.lock";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const TEMP_NAME: &str = "memories.tmp";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const READ_CHUNK_BYTES: usize = 8 * 1_024;

const MEMORY_DESCRIPTION: &str = "Store durable user preferences across sessions. Use save only after an explicit user request. Do not store secrets, credentials, task notes, repository facts, or temporary context";

/// Stable category for failure to acquire a memory state root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryToolOpenErrorKind {
    /// Native durable memory is unavailable on this target.
    UnsupportedPlatform,
    /// The injected root path was not absolute.
    InvalidRoot,
    /// The injected root did not resolve to a real directory.
    InvalidFileType,
    /// The injected root could not be safely retained.
    Unavailable,
}

/// Fixed, redacted failure to construct a [`MemoryTool`].
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct MemoryToolOpenError {
    kind: MemoryToolOpenErrorKind,
}

impl MemoryToolOpenError {
    /// Returns the stable category of this construction failure.
    #[must_use]
    pub const fn kind(&self) -> MemoryToolOpenErrorKind {
        self.kind
    }

    const fn new(kind: MemoryToolOpenErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for MemoryToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MemoryToolOpenError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for MemoryToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            MemoryToolOpenErrorKind::UnsupportedPlatform => {
                "native memory is unsupported on this platform"
            }
            MemoryToolOpenErrorKind::InvalidRoot => "native memory state root is invalid",
            MemoryToolOpenErrorKind::InvalidFileType => {
                "native memory state root is not a directory"
            }
            MemoryToolOpenErrorKind::Unavailable => "native memory state root is unavailable",
        })
    }
}

impl Error for MemoryToolOpenError {}

/// Bounded durable preference storage confined to one retained state root.
///
/// Construction retains only the supplied directory descriptor. Execution is
/// synchronous on the polling thread, starts no detached work, and uses only
/// the three fixed descriptor-relative memory children.
pub struct MemoryTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    root: OwnedFd,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    _unsupported: std::convert::Infallible,
}

impl MemoryTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) const fn from_root_descriptor(root: OwnedFd) -> Self {
        Self { root }
    }

    /// Opens an existing absolute state directory without following its final
    /// component. Construction creates no child, task, or background worker.
    ///
    /// # Errors
    ///
    /// Returns a fixed redacted error when the platform or root is unsuitable.
    pub fn open(root: &Path) -> Result<Self, MemoryToolOpenError> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = root;
            Err(MemoryToolOpenError::new(
                MemoryToolOpenErrorKind::UnsupportedPlatform,
            ))
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let lexical_root = root.components().collect::<std::path::PathBuf>();
            if !lexical_root.is_absolute() {
                return Err(MemoryToolOpenError::new(
                    MemoryToolOpenErrorKind::InvalidRoot,
                ));
            }
            let descriptor = rustix::fs::open(&lexical_root, directory_open_flags(), Mode::empty())
                .map_err(|error| map_root_open_error(&lexical_root, error))?;
            let metadata = rustix::fs::fstat(&descriptor)
                .map_err(|_| MemoryToolOpenError::new(MemoryToolOpenErrorKind::Unavailable))?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(MemoryToolOpenError::new(
                    MemoryToolOpenErrorKind::InvalidFileType,
                ));
            }
            Ok(Self::from_root_descriptor(descriptor))
        }
    }
}

impl fmt::Debug for MemoryTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("MemoryTool").finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum MemoryAction {
    Save { fact: String },
    List,
    Clear,
}

impl MemoryAction {
    fn as_json(&self) -> Value {
        match self {
            Self::Save { fact } => json!({"action": "save", "fact": fact}),
            Self::List => json!({"action": "list"}),
            Self::Clear => json!({"action": "clear"}),
        }
    }
}

impl Tool for MemoryTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: memory_name(),
            description: MEMORY_DESCRIPTION.to_owned(),
            input_schema: memory_input_schema(),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        if call.name != memory_name() {
            return Err(invalid_arguments());
        }
        let action = decode_arguments(&call.arguments)?;
        let arguments = action.as_json();
        ensure_serialized_arguments(&arguments)?;
        Ok(PreparedToolCall::new(
            Capability::Custom {
                name: MEMORY_TOOL_NAME.to_owned(),
                details: arguments.clone(),
            },
            arguments,
        ))
    }

    fn execute(
        &self,
        _context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async move {
            check_cancellation(&cancellation)?;
            let action = decode_arguments(&arguments)?;
            let canonical = action.as_json();
            ensure_serialized_arguments(&canonical)?;
            if canonical != arguments {
                return Err(invalid_arguments());
            }

            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                let _ = (action, cancellation);
                Err(unsupported_platform())
            }

            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                self.execute_supported(action, &cancellation)
            }
        })
    }
}

fn memory_name() -> ToolName {
    ToolName::new(MEMORY_TOOL_NAME).expect("memory is a valid tool name")
}

fn memory_input_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "action": {"const": "save"},
                    "fact": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": MAX_MEMORY_FACT_BYTES,
                        "description": "Exact durable user preference explicitly requested for retention"
                    }
                },
                "required": ["action", "fact"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {"action": {"const": "list"}},
                "required": ["action"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {"action": {"const": "clear"}},
                "required": ["action"],
                "additionalProperties": false
            }
        ]
    })
}

fn decode_arguments(arguments: &Value) -> Result<MemoryAction, ToolError> {
    let Value::Object(object) = arguments else {
        return Err(invalid_arguments());
    };
    let Some(Value::String(action)) = object.get("action") else {
        return Err(invalid_arguments());
    };
    match action.as_str() {
        "save" if object.len() == 2 => {
            let Some(Value::String(fact)) = object.get("fact") else {
                return Err(invalid_arguments());
            };
            validate_fact(fact)?;
            Ok(MemoryAction::Save { fact: fact.clone() })
        }
        "list" if object.len() == 1 => Ok(MemoryAction::List),
        "clear" if object.len() == 1 => Ok(MemoryAction::Clear),
        _ => Err(invalid_arguments()),
    }
}

fn validate_fact(fact: &str) -> Result<(), ToolError> {
    if fact.is_empty() || fact.len() > MAX_MEMORY_FACT_BYTES {
        Err(invalid_fact())
    } else {
        Ok(())
    }
}

fn ensure_serialized_arguments(arguments: &Value) -> Result<(), ToolError> {
    if serialized_value_fits(arguments, MAX_MEMORY_SERIALIZED_ARGUMENT_BYTES) {
        Ok(())
    } else {
        Err(resource_limit())
    }
}

fn serialized_value_fits(value: &(impl Serialize + ?Sized), limit: usize) -> bool {
    let mut counter = JsonByteCounter { written: 0, limit };
    serde_json::to_writer(&mut counter, value).is_ok()
}

struct JsonByteCounter {
    written: usize,
    limit: usize,
}

impl io::Write for JsonByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(written) = self.written.checked_add(bytes.len()) else {
            return Err(io::Error::other("serialized JSON byte count overflowed"));
        };
        if written > self.limit {
            return Err(io::Error::other("serialized JSON exceeded its byte limit"));
        }
        self.written = written;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDocument {
    schema_version: u32,
    memories: Vec<String>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Serialize)]
struct WriteDocument<'a> {
    schema_version: u32,
    memories: &'a [String],
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
enum LoadedMemories {
    Missing,
    Present(Vec<String>),
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl LoadedMemories {
    fn as_slice(&self) -> &[String] {
        match self {
            Self::Missing => &[],
            Self::Present(memories) => memories,
        }
    }

    fn into_vec(self) -> Vec<String> {
        match self {
            Self::Missing => Vec::new(),
            Self::Present(memories) => memories,
        }
    }

    const fn is_present(&self) -> bool {
        matches!(self, Self::Present(_))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Default)]
struct IoBudget {
    attempts: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl IoBudget {
    fn dispatch<T>(
        &mut self,
        cancellation: &CancellationToken,
        operation: impl FnOnce() -> Result<T, rustix::io::Errno>,
    ) -> Result<Result<T, rustix::io::Errno>, ToolError> {
        check_cancellation(cancellation)?;
        self.charge()?;
        Ok(operation())
    }

    fn charge(&mut self) -> Result<(), ToolError> {
        if self.attempts >= MAX_MEMORY_IO_ATTEMPTS {
            return Err(resource_limit());
        }
        self.attempts += 1;
        Ok(())
    }

    fn charge_postcommit(&mut self) -> Result<(), ToolError> {
        if self.attempts >= MAX_MEMORY_IO_ATTEMPTS {
            return Err(commit_ambiguous());
        }
        self.attempts += 1;
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl MemoryTool {
    fn execute_supported(
        &self,
        action: MemoryAction,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        check_cancellation(cancellation)?;
        let mut budget = IoBudget::default();
        let lock = open_lock(self.root.as_fd(), &mut budget, cancellation)?;
        let operation = match action {
            MemoryAction::List => FlockOperation::NonBlockingLockShared,
            MemoryAction::Save { .. } | MemoryAction::Clear => {
                FlockOperation::NonBlockingLockExclusive
            }
        };
        acquire_lock(&lock, operation, &mut budget, cancellation)?;
        check_cancellation(cancellation)?;

        match action {
            MemoryAction::List => {
                let memories = read_memories(self.root.as_fd(), &mut budget, cancellation)?;
                build_list_output(memories.as_slice())
            }
            MemoryAction::Save { fact } => self.save(fact, &mut budget, cancellation),
            MemoryAction::Clear => self.clear(&mut budget, cancellation),
        }
    }

    fn save(
        &self,
        fact: String,
        budget: &mut IoBudget,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        let loaded = read_memories(self.root.as_fd(), budget, cancellation)?;
        if loaded.as_slice().iter().any(|memory| memory == &fact) {
            check_cancellation(cancellation)?;
            return build_save_output(false, loaded.as_slice().len());
        }

        let mut memories = loaded.into_vec();
        memories.try_reserve(1).map_err(|_| memory_unavailable())?;
        memories.push(fact);
        validate_memory_set(&memories).map_err(|error| match error {
            StateValidationError::ResourceLimit => resource_limit(),
            StateValidationError::Corrupt => state_corrupt(),
        })?;
        let bytes = serialize_document(&memories)?;
        // A newly saved state must remain listable within the result envelope.
        let _future_list = build_list_output(&memories)?;
        let output = build_save_output(true, memories.len())?;

        remove_stale_temp(self.root.as_fd(), budget, cancellation)?;
        publish_document(self.root.as_fd(), &bytes, budget, cancellation)?;
        Ok(output)
    }

    fn clear(
        &self,
        budget: &mut IoBudget,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        let loaded = read_memories(self.root.as_fd(), budget, cancellation)?;
        let cleared = loaded.as_slice().len();
        let output = build_clear_output(cleared)?;
        let removed_temp = remove_stale_temp(self.root.as_fd(), budget, cancellation)?;

        if !loaded.is_present() {
            if removed_temp {
                sync_directory_precommit(self.root.as_fd(), budget, cancellation)?;
            }
            check_cancellation(cancellation)?;
            return Ok(output);
        }

        budget.charge()?;
        check_cancellation(cancellation)?;
        match rustix::fs::unlinkat(self.root.as_fd(), DATA_NAME, AtFlags::empty()) {
            Ok(()) => sync_directory_postcommit(self.root.as_fd(), budget)?,
            Err(error) if error == rustix::io::Errno::INTR => return Err(commit_ambiguous()),
            Err(_) => return Err(write_failed()),
        }
        Ok(output)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn open_lock(
    root: BorrowedFd<'_>,
    budget: &mut IoBudget,
    cancellation: &CancellationToken,
) -> Result<OwnedFd, ToolError> {
    let created = budget.dispatch(cancellation, || {
        rustix::fs::openat(
            root,
            LOCK_NAME,
            OFlags::RDWR
                | OFlags::CREATE
                | OFlags::EXCL
                | OFlags::NOFOLLOW
                | OFlags::CLOEXEC
                | OFlags::NONBLOCK,
            Mode::from_raw_mode(0o600),
        )
    })?;
    let (lock, was_created) = match created {
        Ok(lock) => (lock, true),
        Err(error) if error == rustix::io::Errno::EXIST => {
            let opened = budget.dispatch(cancellation, || {
                rustix::fs::openat(
                    root,
                    LOCK_NAME,
                    OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                    Mode::empty(),
                )
            })?;
            let lock = match opened {
                Ok(lock) => lock,
                Err(error) => {
                    return Err(map_existing_child_open_error(
                        root,
                        LOCK_NAME,
                        error,
                        budget,
                        cancellation,
                    )?);
                }
            };
            (lock, false)
        }
        Err(error) => {
            return Err(map_created_child_error(
                root,
                LOCK_NAME,
                error,
                budget,
                cancellation,
            )?);
        }
    };
    ensure_regular(&lock, budget, cancellation)?;
    if was_created {
        budget
            .dispatch(cancellation, || {
                rustix::fs::fchmod(&lock, Mode::RUSR | Mode::WUSR)
            })?
            .map_err(|_| memory_unavailable())?;
    }
    check_cancellation(cancellation)?;
    Ok(lock)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn acquire_lock(
    lock: &OwnedFd,
    operation: FlockOperation,
    budget: &mut IoBudget,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    loop {
        match budget.dispatch(cancellation, || rustix::fs::flock(lock, operation))? {
            Ok(()) => {
                check_cancellation(cancellation)?;
                return Ok(());
            }
            Err(error) if error == rustix::io::Errno::INTR => {}
            Err(error)
                if error == rustix::io::Errno::AGAIN || error == rustix::io::Errno::WOULDBLOCK =>
            {
                return Err(memory_busy());
            }
            Err(_) => return Err(memory_unavailable()),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_memories(
    root: BorrowedFd<'_>,
    budget: &mut IoBudget,
    cancellation: &CancellationToken,
) -> Result<LoadedMemories, ToolError> {
    let opened = budget.dispatch(cancellation, || {
        rustix::fs::openat(
            root,
            DATA_NAME,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
    })?;
    let file = match opened {
        Ok(file) => file,
        Err(error) if error == rustix::io::Errno::NOENT => {
            check_cancellation(cancellation)?;
            return Ok(LoadedMemories::Missing);
        }
        Err(error) => {
            return Err(map_existing_child_open_error(
                root,
                DATA_NAME,
                error,
                budget,
                cancellation,
            )?);
        }
    };
    let metadata = ensure_regular(&file, budget, cancellation)?;
    if metadata.st_size < 0 {
        return Err(state_corrupt());
    }
    let size = usize::try_from(metadata.st_size).map_err(|_| state_corrupt())?;
    if size > MAX_MEMORY_FILE_BYTES {
        return Err(state_corrupt());
    }

    let mut bytes = Vec::with_capacity(size.min(MAX_MEMORY_FILE_BYTES));
    let mut chunk = [0_u8; READ_CHUNK_BYTES];
    loop {
        let remaining = MAX_MEMORY_FILE_BYTES
            .saturating_add(1)
            .saturating_sub(bytes.len());
        if remaining == 0 {
            return Err(state_corrupt());
        }
        let request = remaining.min(chunk.len());
        match budget.dispatch(cancellation, || {
            rustix::io::read(&file, &mut chunk[..request])
        })? {
            Ok(0) => break,
            Ok(read) => {
                bytes.extend_from_slice(&chunk[..read]);
                check_cancellation(cancellation)?;
                if bytes.len() > MAX_MEMORY_FILE_BYTES {
                    return Err(state_corrupt());
                }
            }
            Err(error) if error == rustix::io::Errno::INTR => {}
            Err(_) => return Err(read_failed()),
        }
    }

    let document: StoredDocument = serde_json::from_slice(&bytes).map_err(|_| state_corrupt())?;
    if document.schema_version != MEMORY_SCHEMA_VERSION {
        return Err(state_corrupt());
    }
    validate_memory_set(&document.memories).map_err(|_| state_corrupt())?;
    let compact = serialize_document(&document.memories).map_err(|_| state_corrupt())?;
    if compact.len() > MAX_MEMORY_FILE_BYTES || build_list_output(&document.memories).is_err() {
        return Err(state_corrupt());
    }
    check_cancellation(cancellation)?;
    Ok(LoadedMemories::Present(document.memories))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy)]
enum StateValidationError {
    Corrupt,
    ResourceLimit,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_memory_set(memories: &[String]) -> Result<(), StateValidationError> {
    if memories.len() > MAX_MEMORY_FACTS {
        return Err(StateValidationError::ResourceLimit);
    }
    let mut total = 0_usize;
    for (index, memory) in memories.iter().enumerate() {
        if memory.is_empty() || memory.len() > MAX_MEMORY_FACT_BYTES {
            return Err(StateValidationError::Corrupt);
        }
        total = total
            .checked_add(memory.len())
            .filter(|total| *total <= MAX_MEMORY_TOTAL_FACT_BYTES)
            .ok_or(StateValidationError::ResourceLimit)?;
        if memories[..index].iter().any(|existing| existing == memory) {
            return Err(StateValidationError::Corrupt);
        }
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn serialize_document(memories: &[String]) -> Result<Vec<u8>, ToolError> {
    let mut writer = BoundedJsonWriter::new(MAX_MEMORY_FILE_BYTES);
    serde_json::to_writer(
        &mut writer,
        &WriteDocument {
            schema_version: MEMORY_SCHEMA_VERSION,
            memories,
        },
    )
    .map_err(|_| resource_limit())?;
    Ok(writer.bytes)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct BoundedJsonWriter {
    bytes: Vec<u8>,
    limit: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl BoundedJsonWriter {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl io::Write for BoundedJsonWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let end = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("memory JSON byte count overflowed"))?;
        if end > self.limit {
            return Err(io::Error::other("memory JSON exceeded its byte limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn remove_stale_temp(
    root: BorrowedFd<'_>,
    budget: &mut IoBudget,
    cancellation: &CancellationToken,
) -> Result<bool, ToolError> {
    let opened = budget.dispatch(cancellation, || {
        rustix::fs::openat(
            root,
            TEMP_NAME,
            OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
    })?;
    let temp = match opened {
        Ok(temp) => temp,
        Err(error) if error == rustix::io::Errno::NOENT => return Ok(false),
        Err(error) => {
            return Err(map_existing_child_open_error(
                root,
                TEMP_NAME,
                error,
                budget,
                cancellation,
            )?);
        }
    };
    ensure_regular(&temp, budget, cancellation)?;
    drop(temp);
    unlink_temp(root, budget, cancellation)?;
    Ok(true)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unlink_temp(
    root: BorrowedFd<'_>,
    budget: &mut IoBudget,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    let mut interrupted = false;
    loop {
        match budget.dispatch(cancellation, || {
            rustix::fs::unlinkat(root, TEMP_NAME, AtFlags::empty())
        })? {
            Ok(()) => return Ok(()),
            Err(error) if error == rustix::io::Errno::INTR => interrupted = true,
            Err(error) if interrupted && error == rustix::io::Errno::NOENT => return Ok(()),
            Err(_) => return Err(write_failed()),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn publish_document(
    root: BorrowedFd<'_>,
    bytes: &[u8],
    budget: &mut IoBudget,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    let created = budget.dispatch(cancellation, || {
        rustix::fs::openat(
            root,
            TEMP_NAME,
            OFlags::WRONLY
                | OFlags::CREATE
                | OFlags::EXCL
                | OFlags::NOFOLLOW
                | OFlags::CLOEXEC
                | OFlags::NONBLOCK,
            Mode::from_raw_mode(0o600),
        )
    })?;
    let temp = match created {
        Ok(temp) => temp,
        Err(error) => {
            return Err(map_created_child_error(
                root,
                TEMP_NAME,
                error,
                budget,
                cancellation,
            )?);
        }
    };

    let precommit = (|| {
        ensure_regular(&temp, budget, cancellation)?;
        budget
            .dispatch(cancellation, || {
                rustix::fs::fchmod(&temp, Mode::RUSR | Mode::WUSR)
            })?
            .map_err(|_| write_failed())?;
        write_all(&temp, bytes, budget, cancellation)?;
        sync_temp(&temp, budget, cancellation)?;
        check_cancellation(cancellation)
    })();
    if let Err(error) = precommit {
        best_effort_temp_cleanup(root, budget);
        return Err(error);
    }
    drop(temp);

    if let Err(error) = budget
        .charge()
        .and_then(|()| check_cancellation(cancellation))
    {
        best_effort_temp_cleanup(root, budget);
        return Err(error);
    }
    match rustix::fs::renameat(root, TEMP_NAME, root, DATA_NAME) {
        Ok(()) => sync_directory_postcommit(root, budget),
        Err(error) if error == rustix::io::Errno::INTR => Err(commit_ambiguous()),
        Err(_) => {
            best_effort_temp_cleanup(root, budget);
            Err(write_failed())
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_all(
    file: &OwnedFd,
    mut bytes: &[u8],
    budget: &mut IoBudget,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    while !bytes.is_empty() {
        match budget.dispatch(cancellation, || rustix::io::write(file, bytes))? {
            Ok(0) => return Err(write_failed()),
            Ok(written) => {
                bytes = &bytes[written..];
                check_cancellation(cancellation)?;
            }
            Err(error) if error == rustix::io::Errno::INTR => {}
            Err(_) => return Err(write_failed()),
        }
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_temp(
    file: &OwnedFd,
    budget: &mut IoBudget,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    loop {
        match budget.dispatch(cancellation, || rustix::fs::fsync(file))? {
            Ok(()) => {
                check_cancellation(cancellation)?;
                return Ok(());
            }
            Err(error) if error == rustix::io::Errno::INTR => {}
            Err(_) => return Err(write_failed()),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_directory_precommit(
    root: BorrowedFd<'_>,
    budget: &mut IoBudget,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    loop {
        match budget.dispatch(cancellation, || rustix::fs::fsync(root))? {
            Ok(()) => {
                check_cancellation(cancellation)?;
                return Ok(());
            }
            Err(error) if error == rustix::io::Errno::INTR => {}
            Err(_) => return Err(write_failed()),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_directory_postcommit(root: BorrowedFd<'_>, budget: &mut IoBudget) -> Result<(), ToolError> {
    loop {
        budget.charge_postcommit()?;
        match rustix::fs::fsync(root) {
            Ok(()) => return Ok(()),
            Err(error) if error == rustix::io::Errno::INTR => {}
            Err(_) => return Err(commit_ambiguous()),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn best_effort_temp_cleanup(root: BorrowedFd<'_>, budget: &mut IoBudget) {
    if budget.attempts >= MAX_MEMORY_IO_ATTEMPTS {
        return;
    }
    budget.attempts += 1;
    let _ = rustix::fs::unlinkat(root, TEMP_NAME, AtFlags::empty());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ensure_regular(
    file: &OwnedFd,
    budget: &mut IoBudget,
    cancellation: &CancellationToken,
) -> Result<rustix::fs::Stat, ToolError> {
    let metadata = budget
        .dispatch(cancellation, || rustix::fs::fstat(file))?
        .map_err(|_| memory_unavailable())?;
    if !FileType::from_raw_mode(metadata.st_mode).is_file() {
        return Err(state_corrupt());
    }
    check_cancellation(cancellation)?;
    Ok(metadata)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_created_child_error(
    root: BorrowedFd<'_>,
    name: &str,
    error: rustix::io::Errno,
    budget: &mut IoBudget,
    cancellation: &CancellationToken,
) -> Result<ToolError, ToolError> {
    if error == rustix::io::Errno::EXIST || is_rejected_type_error(error) {
        return Ok(state_corrupt());
    }
    Ok(
        classify_child_after_open_error(root, name, budget, cancellation)?
            .unwrap_or_else(memory_unavailable),
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_existing_child_open_error(
    root: BorrowedFd<'_>,
    name: &str,
    error: rustix::io::Errno,
    budget: &mut IoBudget,
    cancellation: &CancellationToken,
) -> Result<ToolError, ToolError> {
    if is_rejected_type_error(error) {
        return Ok(state_corrupt());
    }
    Ok(
        classify_child_after_open_error(root, name, budget, cancellation)?
            .unwrap_or_else(memory_unavailable),
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn classify_child_after_open_error(
    root: BorrowedFd<'_>,
    name: &str,
    budget: &mut IoBudget,
    cancellation: &CancellationToken,
) -> Result<Option<ToolError>, ToolError> {
    let metadata = budget
        .dispatch(cancellation, || {
            rustix::fs::statat(root, name, AtFlags::SYMLINK_NOFOLLOW)
        })?
        .ok();
    Ok(metadata.and_then(|metadata| {
        (!FileType::from_raw_mode(metadata.st_mode).is_file()).then(state_corrupt)
    }))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_rejected_type_error(error: rustix::io::Errno) -> bool {
    error == rustix::io::Errno::LOOP
        || error == rustix::io::Errno::NOTDIR
        || error == rustix::io::Errno::ISDIR
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn directory_open_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_root_open_error(root: &Path, error: rustix::io::Errno) -> MemoryToolOpenError {
    let kind = if is_rejected_type_error(error)
        || std::fs::symlink_metadata(root).is_ok_and(|metadata| !metadata.file_type().is_dir())
    {
        MemoryToolOpenErrorKind::InvalidFileType
    } else {
        MemoryToolOpenErrorKind::Unavailable
    };
    MemoryToolOpenError::new(kind)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn build_save_output(stored: bool, count: usize) -> Result<ToolOutput, ToolError> {
    bounded_output(json!({"action": "save", "stored": stored, "count": count}))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn build_list_output(memories: &[String]) -> Result<ToolOutput, ToolError> {
    bounded_output(json!({
        "action": "list",
        "memories": memories,
        "count": memories.len()
    }))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn build_clear_output(cleared: usize) -> Result<ToolOutput, ToolError> {
    bounded_output(json!({"action": "clear", "cleared": cleared}))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn bounded_output(content: Value) -> Result<ToolOutput, ToolError> {
    let output = ToolOutput::success(content);
    if serialized_value_fits(&output, MAX_MEMORY_SERIALIZED_RESULT_BYTES) {
        Ok(output)
    } else {
        Err(resource_limit())
    }
}

fn check_cancellation(cancellation: &CancellationToken) -> Result<(), ToolError> {
    if cancellation.is_cancelled() {
        Err(cancelled())
    } else {
        Ok(())
    }
}

fn invalid_arguments() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "memory_invalid_arguments",
        "memory arguments are invalid",
        false,
    )
}

fn invalid_fact() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "memory_invalid_fact",
        "memory fact is invalid",
        false,
    )
}

fn resource_limit() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "memory_resource_limit",
        "memory resource limit was exceeded",
        false,
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported_platform() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "memory_unsupported_platform",
        "native memory is unsupported on this platform",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn memory_busy() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "memory_busy",
        "memory state is busy",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn state_corrupt() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "memory_state_corrupt",
        "memory state is corrupt",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn memory_unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "memory_unavailable",
        "memory state is unavailable",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_failed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "memory_read_failed",
        "memory state could not be read",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_failed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "memory_write_failed",
        "memory state could not be written",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn commit_ambiguous() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "memory_commit_ambiguous",
        "memory state commit is ambiguous",
        false,
    )
}

fn cancelled() -> ToolError {
    ToolError::new(
        ToolErrorKind::Cancelled,
        "memory_cancelled",
        "memory operation was cancelled",
        false,
    )
}
