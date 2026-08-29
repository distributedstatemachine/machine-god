use std::error::Error;
use std::fmt;
use std::io;
use std::path::Path;

use machine_god_core::{
    BoxFuture, CancellationToken, Capability, FilesystemAccess, PreparedToolCall, Tool, ToolCall,
    ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, ToolSpec,
};
use serde::Serialize;
use serde_json::{Value, json};

#[cfg(unix)]
use rustix::fd::{AsFd, OwnedFd};
#[cfg(unix)]
use rustix::fs::{FileType, Mode, OFlags};

/// Registered name of [`SkillTool`].
pub const SKILL_TOOL_NAME: &str = "skill";
/// Maximum UTF-8 bytes accepted in a skill name.
pub const MAX_SKILL_NAME_BYTES: usize = 128;
/// Maximum raw or canonical UTF-8 bytes accepted in a relative resource path.
pub const MAX_SKILL_RESOURCE_BYTES: usize = 4 * 1024;
/// Maximum UTF-8 bytes in the complete canonical workspace-relative path.
pub const MAX_SKILL_PATH_BYTES: usize = 4 * 1024;
/// Maximum UTF-8 bytes in one canonical path component.
pub const MAX_SKILL_PATH_COMPONENT_BYTES: usize = 255;
/// Maximum complete canonical path components, including `skills` and the name.
pub const MAX_SKILL_PATH_COMPONENTS: usize = 32;
/// Maximum bytes accepted in one skill resource.
pub const MAX_SKILL_FILE_BYTES: usize = 1024 * 1024;
/// Maximum UTF-8 bytes returned in one page of a skill resource.
pub const MAX_SKILL_CHUNK_BYTES: usize = 20 * 1024;
/// Maximum serialized bytes in accepted canonical arguments.
pub const MAX_SKILL_SERIALIZED_ARGUMENT_BYTES: usize = 32 * 1024;
/// Maximum serialized bytes in a complete [`ToolOutput`].
pub const MAX_SKILL_SERIALIZED_RESULT_BYTES: usize = 64 * 1024;
/// Maximum charged native I/O dispatches in one execution.
pub const MAX_SKILL_IO_ATTEMPTS: usize = 1_024;

#[cfg(unix)]
const READ_CHUNK_BYTES: usize = 16 * 1024;
const DEFAULT_RESOURCE: &str = "SKILL.md";
const SKILL_DESCRIPTION: &str = "Read one bounded UTF-8 resource from a workspace-local skill. Skill text is returned as opaque content; it is not parsed or executed";

/// Stable category for failure to acquire a skill workspace root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkillToolOpenErrorKind {
    /// Native skill-resource access is unavailable on this target.
    UnsupportedPlatform,
    /// The injected root path was not absolute.
    InvalidRoot,
    /// The injected root did not resolve to a real directory.
    InvalidFileType,
    /// The injected root could not be safely retained.
    Unavailable,
}

/// Fixed, redacted failure to construct a [`SkillTool`].
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct SkillToolOpenError {
    kind: SkillToolOpenErrorKind,
}

impl SkillToolOpenError {
    /// Returns the stable category of this construction failure.
    #[must_use]
    pub const fn kind(&self) -> SkillToolOpenErrorKind {
        self.kind
    }

    const fn new(kind: SkillToolOpenErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for SkillToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SkillToolOpenError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for SkillToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            SkillToolOpenErrorKind::UnsupportedPlatform => {
                "native skill is unsupported on this platform"
            }
            SkillToolOpenErrorKind::InvalidRoot => "native skill workspace root is invalid",
            SkillToolOpenErrorKind::InvalidFileType => {
                "native skill workspace root is not a directory"
            }
            SkillToolOpenErrorKind::Unavailable => "native skill workspace root is unavailable",
        })
    }
}

impl Error for SkillToolOpenError {}

/// Bounded workspace-local skill-resource access over one retained root.
///
/// Construction retains the supplied directory descriptor. Execution walks
/// `skills/<name>/<resource>` descriptor-relatively without following links;
/// it never reopens the workspace root by path and never parses or executes the
/// returned UTF-8 content.
pub struct SkillTool {
    #[cfg(unix)]
    root: OwnedFd,
    #[cfg(not(unix))]
    _unsupported: std::convert::Infallible,
}

impl SkillTool {
    #[cfg(unix)]
    pub(crate) const fn from_root_descriptor(root: OwnedFd) -> Self {
        Self { root }
    }

    /// Opens and retains an existing absolute workspace directory without
    /// following its final component.
    ///
    /// # Errors
    ///
    /// Returns a fixed redacted error when the platform or root is unsuitable.
    pub fn open(root: &Path) -> Result<Self, SkillToolOpenError> {
        #[cfg(not(unix))]
        {
            let _ = root;
            Err(SkillToolOpenError::new(
                SkillToolOpenErrorKind::UnsupportedPlatform,
            ))
        }

        #[cfg(unix)]
        {
            let lexical_root = root.components().collect::<std::path::PathBuf>();
            if !lexical_root.is_absolute() {
                return Err(SkillToolOpenError::new(SkillToolOpenErrorKind::InvalidRoot));
            }
            let descriptor = rustix::fs::open(&lexical_root, directory_open_flags(), Mode::empty())
                .map_err(map_root_open_error)?;
            let metadata = rustix::fs::fstat(&descriptor)
                .map_err(|_| SkillToolOpenError::new(SkillToolOpenErrorKind::Unavailable))?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(SkillToolOpenError::new(
                    SkillToolOpenErrorKind::InvalidFileType,
                ));
            }
            Ok(Self::from_root_descriptor(descriptor))
        }
    }
}

impl fmt::Debug for SkillTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("SkillTool").finish_non_exhaustive()
    }
}

struct RequestedArguments {
    name: String,
    resource: Option<String>,
    offset: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExecutionArguments {
    name: String,
    resource: String,
    offset: usize,
}

impl ExecutionArguments {
    fn as_json(&self) -> Value {
        json!({
            "name": self.name,
            "resource": self.resource,
            "offset": self.offset,
        })
    }

    fn capability_path(&self) -> String {
        format!("skills/{}/{}", self.name, self.resource)
    }
}

impl Tool for SkillTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: skill_name(),
            description: SKILL_DESCRIPTION.to_owned(),
            input_schema: skill_input_schema(),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        if call.name != skill_name() {
            return Err(invalid_arguments());
        }
        let requested = decode_requested_arguments(call.arguments)?;
        let arguments = ExecutionArguments {
            name: normalize_name(&requested.name)?,
            resource: normalize_resource(
                requested.resource.as_deref().unwrap_or(DEFAULT_RESOURCE),
            )?,
            offset: requested.offset.unwrap_or(0),
        };
        validate_offset_limit(arguments.offset)?;
        validate_complete_path(&arguments)?;
        let prepared = arguments.as_json();
        ensure_serialized_arguments(&prepared)?;
        Ok(PreparedToolCall::new(
            Capability::Filesystem {
                access: FilesystemAccess::Read,
                path: arguments.capability_path(),
            },
            prepared,
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
            let decoded = decode_execution_arguments(&arguments)?;
            validate_canonical_arguments(&decoded)?;
            ensure_serialized_arguments(&arguments)?;

            #[cfg(not(unix))]
            {
                let _ = (decoded, cancellation);
                Err(unsupported_platform())
            }

            #[cfg(unix)]
            {
                self.execute_unix(&decoded, &cancellation)
            }
        })
    }
}

fn skill_name() -> ToolName {
    ToolName::new(SKILL_TOOL_NAME).expect("skill is a valid tool name")
}

fn skill_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_SKILL_NAME_BYTES,
                "description": "Workspace-local skill directory name"
            },
            "resource": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_SKILL_RESOURCE_BYTES,
                "description": "Relative text resource below the skill directory; defaults to SKILL.md"
            },
            "offset": {
                "type": "integer",
                "minimum": 0,
                "maximum": MAX_SKILL_FILE_BYTES,
                "description": "UTF-8 byte offset at which to continue reading; defaults to zero"
            }
        },
        "required": ["name"],
        "additionalProperties": false
    })
}

fn decode_requested_arguments(arguments: Value) -> Result<RequestedArguments, ToolError> {
    let Value::Object(mut object) = arguments else {
        return Err(invalid_arguments());
    };
    if object.is_empty() || object.len() > 3 {
        return Err(invalid_arguments());
    }
    let Some(Value::String(name)) = object.remove("name") else {
        return Err(invalid_arguments());
    };
    let resource = match object.remove("resource") {
        Some(Value::String(resource)) => Some(resource),
        Some(_) => return Err(invalid_arguments()),
        None => None,
    };
    let offset = match object.remove("offset") {
        Some(Value::Number(offset)) => Some(decode_offset(&offset)?),
        Some(_) => return Err(invalid_offset()),
        None => None,
    };
    if !object.is_empty() {
        return Err(invalid_arguments());
    }
    Ok(RequestedArguments {
        name,
        resource,
        offset,
    })
}

fn decode_execution_arguments(arguments: &Value) -> Result<ExecutionArguments, ToolError> {
    let Value::Object(object) = arguments else {
        return Err(invalid_arguments());
    };
    if object.len() != 3 {
        return Err(invalid_arguments());
    }
    let Some(Value::String(name)) = object.get("name") else {
        return Err(invalid_arguments());
    };
    let Some(Value::String(resource)) = object.get("resource") else {
        return Err(invalid_arguments());
    };
    let Some(Value::Number(offset)) = object.get("offset") else {
        return Err(invalid_arguments());
    };
    Ok(ExecutionArguments {
        name: name.clone(),
        resource: resource.clone(),
        offset: decode_offset(offset)?,
    })
}

fn decode_offset(offset: &serde_json::Number) -> Result<usize, ToolError> {
    let value = offset.as_u64().ok_or_else(invalid_arguments)?;
    usize::try_from(value).map_err(|_| invalid_offset())
}

fn validate_canonical_arguments(arguments: &ExecutionArguments) -> Result<(), ToolError> {
    if normalize_name(&arguments.name)? != arguments.name
        || normalize_resource(&arguments.resource)? != arguments.resource
    {
        return Err(invalid_arguments());
    }
    validate_offset_limit(arguments.offset)?;
    validate_complete_path(arguments)
}

fn normalize_name(name: &str) -> Result<String, ToolError> {
    if name.is_empty()
        || name.len() > MAX_SKILL_NAME_BYTES
        || matches!(name, "." | "..")
        || name.chars().any(|character| {
            character == '/' || character == '\\' || forbidden_character(character)
        })
    {
        return Err(invalid_name());
    }
    Ok(name.to_owned())
}

fn normalize_resource(resource: &str) -> Result<String, ToolError> {
    if resource.is_empty()
        || resource.len() > MAX_SKILL_RESOURCE_BYTES
        || resource.starts_with('/')
        || resource
            .chars()
            .any(|character| character == '\\' || forbidden_character(character))
    {
        return Err(invalid_resource());
    }
    let mut normalized = String::with_capacity(resource.len());
    let mut components = 0_usize;
    for component in resource.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            return Err(invalid_resource());
        }
        if component.len() > MAX_SKILL_PATH_COMPONENT_BYTES {
            return Err(invalid_resource());
        }
        components = components.checked_add(1).ok_or_else(resource_limit)?;
        if components > MAX_SKILL_PATH_COMPONENTS.saturating_sub(2) {
            return Err(invalid_resource());
        }
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(component);
    }
    if normalized.is_empty() || normalized.len() > MAX_SKILL_RESOURCE_BYTES {
        return Err(invalid_resource());
    }
    Ok(normalized)
}

fn validate_complete_path(arguments: &ExecutionArguments) -> Result<(), ToolError> {
    if arguments.name.len() > MAX_SKILL_PATH_COMPONENT_BYTES
        || arguments.capability_path().len() > MAX_SKILL_PATH_BYTES
    {
        Err(invalid_resource())
    } else {
        Ok(())
    }
}

fn forbidden_character(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
}

fn validate_offset_limit(offset: usize) -> Result<(), ToolError> {
    if offset > MAX_SKILL_FILE_BYTES {
        Err(invalid_offset())
    } else {
        Ok(())
    }
}

fn ensure_serialized_arguments(arguments: &Value) -> Result<(), ToolError> {
    if serialized_value_fits(arguments, MAX_SKILL_SERIALIZED_ARGUMENT_BYTES) {
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

#[cfg(unix)]
#[derive(Default)]
struct IoBudget {
    attempts: usize,
}

#[cfg(unix)]
impl IoBudget {
    fn dispatch<T>(
        &mut self,
        cancellation: &CancellationToken,
        operation: impl FnOnce() -> Result<T, rustix::io::Errno>,
    ) -> Result<Result<T, rustix::io::Errno>, ToolError> {
        check_cancellation(cancellation)?;
        if self.attempts >= MAX_SKILL_IO_ATTEMPTS {
            return Err(resource_limit());
        }
        self.attempts += 1;
        let result = operation();
        check_cancellation(cancellation)?;
        Ok(result)
    }
}

#[cfg(unix)]
impl SkillTool {
    fn execute_unix(
        &self,
        arguments: &ExecutionArguments,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        let mut budget = IoBudget::default();
        let file = open_resource(self.root.as_fd(), arguments, &mut budget, cancellation)?;
        let bytes = read_resource(&file, &mut budget, cancellation)?;
        let content = String::from_utf8(bytes).map_err(|_| not_utf8())?;
        check_cancellation(cancellation)?;
        render_output(arguments, &content, cancellation)
    }
}

#[cfg(unix)]
fn open_resource(
    root: rustix::fd::BorrowedFd<'_>,
    arguments: &ExecutionArguments,
    budget: &mut IoBudget,
    cancellation: &CancellationToken,
) -> Result<OwnedFd, ToolError> {
    let skills = budget
        .dispatch(cancellation, || {
            rustix::fs::openat(root, "skills", directory_open_flags(), Mode::empty())
        })?
        .map_err(map_path_open_error)?;
    let skill = budget
        .dispatch(cancellation, || {
            rustix::fs::openat(
                skills.as_fd(),
                arguments.name.as_str(),
                directory_open_flags(),
                Mode::empty(),
            )
        })?
        .map_err(map_path_open_error)?;

    let mut directory = skill;
    let mut components = arguments.resource.split('/').peekable();
    loop {
        let component = components.next().ok_or_else(invalid_arguments)?;
        if components.peek().is_some() {
            directory = budget
                .dispatch(cancellation, || {
                    rustix::fs::openat(
                        directory.as_fd(),
                        component,
                        directory_open_flags(),
                        Mode::empty(),
                    )
                })?
                .map_err(map_path_open_error)?;
        } else {
            return budget
                .dispatch(cancellation, || {
                    rustix::fs::openat(
                        directory.as_fd(),
                        component,
                        file_open_flags(),
                        Mode::empty(),
                    )
                })?
                .map_err(map_path_open_error);
        }
    }
}

#[cfg(unix)]
fn read_resource(
    file: &OwnedFd,
    budget: &mut IoBudget,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, ToolError> {
    let metadata = budget
        .dispatch(cancellation, || rustix::fs::fstat(file))?
        .map_err(|_| unavailable(true))?;
    if !FileType::from_raw_mode(metadata.st_mode).is_file() {
        return Err(path_rejected());
    }
    if u64::try_from(metadata.st_size).is_ok_and(|size| size > MAX_SKILL_FILE_BYTES as u64) {
        return Err(resource_limit());
    }

    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(MAX_SKILL_FILE_BYTES + 1)
        .map_err(|_| resource_limit())?;
    let mut chunk = [0_u8; READ_CHUNK_BYTES];
    loop {
        let remaining = MAX_SKILL_FILE_BYTES
            .checked_add(1)
            .and_then(|limit| limit.checked_sub(bytes.len()))
            .ok_or_else(resource_limit)?;
        if remaining == 0 {
            return Err(resource_limit());
        }
        let requested = remaining.min(chunk.len());
        match budget.dispatch(cancellation, || {
            rustix::io::read(file, &mut chunk[..requested])
        })? {
            Ok(0) => break,
            Ok(read) => {
                bytes.extend_from_slice(&chunk[..read]);
                if bytes.len() > MAX_SKILL_FILE_BYTES {
                    return Err(resource_limit());
                }
            }
            Err(error) if error == rustix::io::Errno::INTR => {}
            Err(_) => return Err(read_failed()),
        }
    }
    check_cancellation(cancellation)?;
    Ok(bytes)
}

#[cfg(unix)]
fn render_output(
    arguments: &ExecutionArguments,
    content: &str,
    cancellation: &CancellationToken,
) -> Result<ToolOutput, ToolError> {
    if arguments.offset > content.len() || !content.is_char_boundary(arguments.offset) {
        return Err(invalid_offset());
    }
    let output = fit_output_page(arguments, content, cancellation)?;
    check_cancellation(cancellation)?;
    Ok(output)
}

#[cfg(unix)]
fn fit_output_page(
    arguments: &ExecutionArguments,
    content: &str,
    cancellation: &CancellationToken,
) -> Result<ToolOutput, ToolError> {
    let remaining = content
        .len()
        .checked_sub(arguments.offset)
        .ok_or_else(invalid_offset)?;
    let mut raw_end = arguments
        .offset
        .checked_add(remaining.min(MAX_SKILL_CHUNK_BYTES))
        .ok_or_else(resource_limit)?;
    while !content.is_char_boundary(raw_end) {
        raw_end = raw_end.checked_sub(1).ok_or_else(resource_limit)?;
    }
    let mut boundaries = Vec::new();
    boundaries
        .try_reserve(MAX_SKILL_CHUNK_BYTES.saturating_add(1))
        .map_err(|_| resource_limit())?;
    boundaries.push(arguments.offset);
    for (relative, character) in content[arguments.offset..raw_end].char_indices() {
        let end = arguments
            .offset
            .checked_add(relative)
            .and_then(|value| value.checked_add(character.len_utf8()))
            .ok_or_else(resource_limit)?;
        boundaries.push(end);
    }
    debug_assert_eq!(boundaries.last(), Some(&raw_end));

    let mut fitting = 0_usize;
    let mut excluded = boundaries.len().checked_sub(1).ok_or_else(resource_limit)?;
    while fitting < excluded {
        check_cancellation(cancellation)?;
        let candidate = fitting
            .checked_add(excluded)
            .and_then(|sum| sum.checked_add(1))
            .ok_or_else(resource_limit)?
            / 2;
        if output_fits(arguments, content, boundaries[candidate]) {
            fitting = candidate;
        } else {
            excluded = candidate.checked_sub(1).ok_or_else(resource_limit)?;
        }
    }
    if fitting == 0 && remaining != 0 {
        return Err(resource_limit());
    }
    let end = boundaries[fitting];
    let output = build_output(arguments, content, end);
    if !serialized_value_fits(&output, MAX_SKILL_SERIALIZED_RESULT_BYTES) {
        return Err(resource_limit());
    }
    Ok(output)
}

#[cfg(unix)]
fn output_fits(arguments: &ExecutionArguments, content: &str, end: usize) -> bool {
    serialized_value_fits(
        &build_output(arguments, content, end),
        MAX_SKILL_SERIALIZED_RESULT_BYTES,
    )
}

#[cfg(unix)]
fn build_output(arguments: &ExecutionArguments, content: &str, end: usize) -> ToolOutput {
    ToolOutput::success(json!({
        "name": arguments.name,
        "resource": arguments.resource,
        "offset": arguments.offset,
        "next_offset": end,
        "total_bytes": content.len(),
        "content": &content[arguments.offset..end],
        "truncated": end < content.len(),
    }))
}

#[cfg(unix)]
const fn directory_open_flags() -> OFlags {
    OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC)
        .union(OFlags::NONBLOCK)
}

#[cfg(unix)]
const fn file_open_flags() -> OFlags {
    OFlags::RDONLY
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC)
        .union(OFlags::NONBLOCK)
}

#[cfg(unix)]
fn map_root_open_error(error: rustix::io::Errno) -> SkillToolOpenError {
    let kind = if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        SkillToolOpenErrorKind::InvalidFileType
    } else {
        SkillToolOpenErrorKind::Unavailable
    };
    SkillToolOpenError::new(kind)
}

#[cfg(unix)]
fn map_path_open_error(error: rustix::io::Errno) -> ToolError {
    if error == rustix::io::Errno::NOENT {
        not_found()
    } else if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        path_rejected()
    } else if error == rustix::io::Errno::ACCESS || error == rustix::io::Errno::PERM {
        permission_denied()
    } else {
        unavailable(true)
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
        "skill_invalid_arguments",
        "skill arguments are invalid",
        false,
    )
}

fn invalid_name() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "skill_invalid_name",
        "skill name is invalid",
        false,
    )
}

fn invalid_resource() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "skill_invalid_resource",
        "skill resource is invalid",
        false,
    )
}

fn invalid_offset() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "skill_invalid_offset",
        "skill offset is invalid",
        false,
    )
}

fn resource_limit() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "skill_resource_limit",
        "skill resource limit was exceeded",
        false,
    )
}

#[cfg(not(unix))]
fn unsupported_platform() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "skill_unsupported_platform",
        "native skill is unsupported on this platform",
        false,
    )
}

#[cfg(unix)]
fn not_found() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "skill_not_found",
        "requested skill resource is unavailable",
        false,
    )
}

#[cfg(unix)]
fn permission_denied() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "skill_permission_denied",
        "requested skill resource cannot be read",
        false,
    )
}

#[cfg(unix)]
fn path_rejected() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "skill_path_rejected",
        "requested skill resource is not confined",
        false,
    )
}

#[cfg(unix)]
fn unavailable(retryable: bool) -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "skill_unavailable",
        "requested skill resource is unavailable",
        retryable,
    )
}

#[cfg(unix)]
fn read_failed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "skill_read_failed",
        "requested skill resource could not be read",
        true,
    )
}

#[cfg(unix)]
fn not_utf8() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "skill_not_utf8",
        "requested skill resource is not valid UTF-8",
        false,
    )
}

fn cancelled() -> ToolError {
    ToolError::new(
        ToolErrorKind::Cancelled,
        "skill_cancelled",
        "skill execution was cancelled",
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        ExecutionArguments, IoBudget, MAX_SKILL_CHUNK_BYTES, MAX_SKILL_FILE_BYTES,
        MAX_SKILL_IO_ATTEMPTS, fit_output_page, normalize_name, normalize_resource,
        validate_canonical_arguments,
    };
    use machine_god_core::CancellationToken;

    #[test]
    fn lexical_validation_is_strict_and_canonical() {
        assert_eq!(normalize_name("rust").unwrap(), "rust");
        assert_eq!(
            normalize_resource("./references//guide.md").unwrap(),
            "references/guide.md"
        );
        for name in ["", ".", "..", "a/b", "a\\b", "a\u{202e}b"] {
            assert!(normalize_name(name).is_err(), "accepted {name:?}");
        }
        for resource in ["", ".", "..", "/SKILL.md", "../x", "a/../b", "a\\b"] {
            assert!(
                normalize_resource(resource).is_err(),
                "accepted {resource:?}"
            );
        }
    }

    #[test]
    fn execution_arguments_must_be_canonical_and_bounded() {
        let canonical = ExecutionArguments {
            name: "rust".to_owned(),
            resource: "SKILL.md".to_owned(),
            offset: MAX_SKILL_FILE_BYTES,
        };
        assert!(validate_canonical_arguments(&canonical).is_ok());
        let noncanonical = ExecutionArguments {
            resource: "./SKILL.md".to_owned(),
            ..canonical
        };
        assert!(validate_canonical_arguments(&noncanonical).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn page_boundaries_preserve_utf8_and_shrink_for_json_escaping() {
        let arguments = ExecutionArguments {
            name: "rust".to_owned(),
            resource: "SKILL.md".to_owned(),
            offset: 0,
        };
        let cancellation = CancellationToken::new();
        let text = "\0".repeat(MAX_SKILL_CHUNK_BYTES + 1);
        let output = fit_output_page(&arguments, &text, &cancellation).unwrap();
        let next = output.content["next_offset"].as_u64().unwrap() as usize;
        assert!(next < MAX_SKILL_CHUNK_BYTES);
        assert!(next > 0);
        assert!(text.is_char_boundary(next));
        assert_eq!(output.content["truncated"], true);
    }

    #[cfg(unix)]
    #[test]
    fn io_attempt_limit_is_deterministic() {
        let mut budget = IoBudget {
            attempts: MAX_SKILL_IO_ATTEMPTS,
        };
        let cancellation = CancellationToken::new();
        let error = budget
            .dispatch(&cancellation, || Ok::<_, rustix::io::Errno>(()))
            .unwrap_err();
        assert_eq!(error.code, "skill_resource_limit");
    }
}
