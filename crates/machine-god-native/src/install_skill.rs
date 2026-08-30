use std::error::Error;
use std::fmt;
use std::path::Path;

use machine_god_core::{
    BoxFuture, CancellationToken, Capability, PreparedToolCall, Tool, ToolCall, ToolContext,
    ToolError, ToolErrorKind, ToolName, ToolOutput, ToolSpec,
};
use serde_json::{Value, json};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, RenameFlags, SeekFrom};

/// Registered name of [`InstallSkillTool`].
pub const INSTALL_SKILL_TOOL_NAME: &str = "install_skill";
/// Maximum UTF-8 bytes accepted in the source path.
pub const MAX_INSTALL_SKILL_SOURCE_BYTES: usize = 4 * 1024;
/// Maximum UTF-8 bytes in one admitted descendant path.
pub const MAX_INSTALL_SKILL_PATH_BYTES: usize = 4 * 1024;
/// Maximum UTF-8 bytes accepted in the derived skill name.
pub const MAX_INSTALL_SKILL_NAME_BYTES: usize = 128;
/// Maximum bytes in one path component.
pub const MAX_INSTALL_SKILL_COMPONENT_BYTES: usize = 255;
/// Maximum source-tree depth and source path components.
pub const MAX_INSTALL_SKILL_PATH_COMPONENTS: usize = 32;
/// Maximum admitted entries below the source directory.
pub const MAX_INSTALL_SKILL_ENTRIES: usize = 256;
/// Maximum bytes admitted from one regular file.
pub const MAX_INSTALL_SKILL_FILE_BYTES: usize = 1024 * 1024;
/// Maximum aggregate regular-file bytes admitted from one source tree.
pub const MAX_INSTALL_SKILL_TOTAL_BYTES: usize = 8 * 1024 * 1024;
/// Maximum aggregate UTF-8 bytes in admitted entry names.
pub const MAX_INSTALL_SKILL_ENTRY_NAME_BYTES: usize = 1024 * 1024;
/// Maximum bytes transferred by one native read or write.
pub const MAX_INSTALL_SKILL_CHUNK_BYTES: usize = 64 * 1024;
/// Maximum charged native operations in one execution.
pub const MAX_INSTALL_SKILL_IO_ATTEMPTS: usize = 8 * 1024;
/// Maximum exclusive staging-name attempts.
pub const MAX_INSTALL_SKILL_STAGE_ATTEMPTS: usize = 8;
/// Maximum serialized canonical argument bytes.
pub const MAX_INSTALL_SKILL_SERIALIZED_ARGUMENT_BYTES: usize = 32 * 1024;
/// Maximum serialized [`ToolOutput`] bytes.
pub const MAX_INSTALL_SKILL_SERIALIZED_RESULT_BYTES: usize = 16 * 1024;

const DESCRIPTION: &str = "Install one bounded workspace-local skill directory into skills/<name> without interpreting its contents";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const STAGE_PREFIX: &str = ".machine-god-install-skill-";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_POSTCOMMIT_SYNC_ATTEMPTS: usize = 16;

/// Stable category for failure to acquire an install-capable workspace root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallSkillToolOpenErrorKind {
    /// Native installation is unavailable on this target.
    UnsupportedPlatform,
    /// The supplied root was not absolute.
    InvalidRoot,
    /// The supplied root was not a real directory.
    InvalidFileType,
    /// The supplied root could not be retained.
    Unavailable,
}

/// Fixed redacted construction failure for [`InstallSkillTool`].
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct InstallSkillToolOpenError {
    kind: InstallSkillToolOpenErrorKind,
}

impl InstallSkillToolOpenError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> InstallSkillToolOpenErrorKind {
        self.kind
    }

    const fn new(kind: InstallSkillToolOpenErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for InstallSkillToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstallSkillToolOpenError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for InstallSkillToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            InstallSkillToolOpenErrorKind::UnsupportedPlatform => {
                "native install_skill is unsupported on this platform"
            }
            InstallSkillToolOpenErrorKind::InvalidRoot => {
                "native install_skill workspace root is invalid"
            }
            InstallSkillToolOpenErrorKind::InvalidFileType => {
                "native install_skill workspace root is not a directory"
            }
            InstallSkillToolOpenErrorKind::Unavailable => {
                "native install_skill workspace root is unavailable"
            }
        })
    }
}

impl Error for InstallSkillToolOpenError {}

/// Bounded local skill installer confined to one retained workspace root.
pub struct InstallSkillTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    root: OwnedFd,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    _unsupported: std::convert::Infallible,
}

impl InstallSkillTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) const fn from_root_descriptor(root: OwnedFd) -> Self {
        Self { root }
    }

    /// Opens and retains one existing absolute workspace directory.
    ///
    /// # Errors
    ///
    /// Returns a fixed redacted error when the target or root is unsuitable.
    pub fn open(root: &Path) -> Result<Self, InstallSkillToolOpenError> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = root;
            Err(InstallSkillToolOpenError::new(
                InstallSkillToolOpenErrorKind::UnsupportedPlatform,
            ))
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let lexical = root.components().collect::<std::path::PathBuf>();
            if !lexical.is_absolute() {
                return Err(InstallSkillToolOpenError::new(
                    InstallSkillToolOpenErrorKind::InvalidRoot,
                ));
            }
            let descriptor = rustix::fs::open(&lexical, directory_flags(), Mode::empty())
                .map_err(map_root_open_error)?;
            let metadata = rustix::fs::fstat(&descriptor).map_err(|_| {
                InstallSkillToolOpenError::new(InstallSkillToolOpenErrorKind::Unavailable)
            })?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(InstallSkillToolOpenError::new(
                    InstallSkillToolOpenErrorKind::InvalidFileType,
                ));
            }
            Ok(Self::from_root_descriptor(descriptor))
        }
    }
}

impl fmt::Debug for InstallSkillTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstallSkillTool")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Arguments {
    source: String,
    skill: String,
}

impl Arguments {
    fn destination(&self) -> String {
        format!("skills/{}", self.skill)
    }

    fn as_json(&self) -> Value {
        json!({"source": self.source, "skill": self.skill})
    }
}

impl Tool for InstallSkillTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: install_skill_name(),
            description: DESCRIPTION.to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "source": {"type": "string", "description": "Workspace-relative source skill directory"},
                    "skill": {"type": "string", "description": "Optional exact source basename confirmation"}
                },
                "required": ["source"],
                "additionalProperties": false
            }),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        if call.name != install_skill_name() {
            return Err(invalid_arguments());
        }
        let arguments = decode_arguments(call.arguments, false)?;
        let canonical = arguments.as_json();
        ensure_serialized(&canonical, MAX_INSTALL_SKILL_SERIALIZED_ARGUMENT_BYTES)?;
        Ok(PreparedToolCall::new(
            Capability::Custom {
                name: INSTALL_SKILL_TOOL_NAME.to_owned(),
                details: json!({
                    "source": arguments.source,
                    "destination": arguments.destination(),
                }),
            },
            canonical,
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
            let decoded = decode_arguments(arguments.clone(), true)?;
            if decoded.as_json() != arguments {
                return Err(invalid_arguments());
            }
            ensure_serialized(&arguments, MAX_INSTALL_SKILL_SERIALIZED_ARGUMENT_BYTES)?;

            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                let _ = decoded;
                Err(unsupported_platform())
            }

            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                self.execute_native(&decoded, &cancellation)
            }
        })
    }
}

fn install_skill_name() -> ToolName {
    ToolName::new(INSTALL_SKILL_TOOL_NAME).expect("install_skill is a valid tool name")
}

fn decode_arguments(value: Value, canonical: bool) -> Result<Arguments, ToolError> {
    let Value::Object(mut object) = value else {
        return Err(invalid_arguments());
    };
    if object.is_empty() || object.len() > 2 {
        return Err(invalid_arguments());
    }
    let Some(Value::String(source)) = object.remove("source") else {
        return Err(invalid_arguments());
    };
    let supplied_skill = match object.remove("skill") {
        Some(Value::String(skill)) => Some(skill),
        Some(_) => return Err(invalid_arguments()),
        None if canonical => return Err(invalid_arguments()),
        None => None,
    };
    if !object.is_empty() {
        return Err(invalid_arguments());
    }
    let source = normalize_source(&source)?;
    let skill = source
        .rsplit('/')
        .next()
        .ok_or_else(invalid_source)?
        .to_owned();
    validate_skill_name(&skill)?;
    if supplied_skill
        .as_deref()
        .is_some_and(|value| value != skill)
    {
        return Err(invalid_skill());
    }
    if source
        .split('/')
        .next()
        .is_some_and(|component| component.eq_ignore_ascii_case("skills"))
    {
        return Err(overlap());
    }
    Ok(Arguments { source, skill })
}

fn normalize_source(source: &str) -> Result<String, ToolError> {
    if source.is_empty()
        || source.len() > MAX_INSTALL_SKILL_SOURCE_BYTES
        || source.starts_with('/')
        || source.chars().any(forbidden_character)
    {
        return Err(invalid_source());
    }
    let mut normalized = String::with_capacity(source.len());
    let mut count = 0_usize;
    for component in source.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." || component.len() > MAX_INSTALL_SKILL_COMPONENT_BYTES {
            return Err(invalid_source());
        }
        count = count.checked_add(1).ok_or_else(resource_limit)?;
        if count > MAX_INSTALL_SKILL_PATH_COMPONENTS {
            return Err(invalid_source());
        }
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(component);
    }
    if normalized.is_empty() || normalized.len() > MAX_INSTALL_SKILL_SOURCE_BYTES {
        return Err(invalid_source());
    }
    Ok(normalized)
}

fn validate_skill_name(name: &str) -> Result<(), ToolError> {
    if name.is_empty()
        || name.len() > MAX_INSTALL_SKILL_NAME_BYTES
        || name.len() > MAX_INSTALL_SKILL_COMPONENT_BYTES
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.chars().any(forbidden_character)
    {
        return Err(invalid_skill());
    }
    Ok(())
}

fn forbidden_character(character: char) -> bool {
    character == '\\'
        || character.is_control()
        || matches!(character, '\u{007f}'..='\u{009f}' | '\u{2028}' | '\u{2029}')
        || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn ensure_serialized(value: &Value, limit: usize) -> Result<(), ToolError> {
    if serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= limit) {
        Ok(())
    } else {
        Err(resource_limit())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Debug)]
enum PlannedEntryKind {
    Directory,
    File { descriptor: OwnedFd, bytes: usize },
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Debug)]
struct PlannedEntry {
    path: String,
    kind: PlannedEntryKind,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct Budget {
    operations: usize,
    entries: usize,
    name_bytes: usize,
    total_bytes: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Budget {
    const fn new() -> Self {
        Self {
            operations: 0,
            entries: 0,
            name_bytes: 0,
            total_bytes: 0,
        }
    }

    fn charge_io(&mut self) -> Result<(), ToolError> {
        self.operations = self.operations.checked_add(1).ok_or_else(resource_limit)?;
        if self.operations > MAX_INSTALL_SKILL_IO_ATTEMPTS {
            Err(resource_limit())
        } else {
            Ok(())
        }
    }

    fn admit_entry(&mut self, name_bytes: usize) -> Result<(), ToolError> {
        self.entries = self.entries.checked_add(1).ok_or_else(resource_limit)?;
        self.name_bytes = self
            .name_bytes
            .checked_add(name_bytes)
            .ok_or_else(resource_limit)?;
        if self.entries > MAX_INSTALL_SKILL_ENTRIES
            || self.name_bytes > MAX_INSTALL_SKILL_ENTRY_NAME_BYTES
        {
            Err(resource_limit())
        } else {
            Ok(())
        }
    }

    fn admit_file(&mut self, bytes: usize) -> Result<(), ToolError> {
        self.total_bytes = self
            .total_bytes
            .checked_add(bytes)
            .ok_or_else(resource_limit)?;
        if bytes > MAX_INSTALL_SKILL_FILE_BYTES || self.total_bytes > MAX_INSTALL_SKILL_TOTAL_BYTES
        {
            Err(resource_limit())
        } else {
            Ok(())
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl InstallSkillTool {
    fn execute_native(
        &self,
        arguments: &Arguments,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        let mut budget = Budget::new();
        let source = open_relative_directory(
            self.root.as_fd(),
            &arguments.source,
            &mut budget,
            cancellation,
        )?;
        let mut entries = Vec::new();
        scan_directory(
            source.as_fd(),
            "",
            0,
            &mut entries,
            &mut budget,
            cancellation,
        )?;
        let manifest_index = entries
            .iter()
            .position(|entry| entry.path == "SKILL.md")
            .ok_or_else(invalid_manifest)?;
        let PlannedEntryKind::File { descriptor, bytes } = &entries[manifest_index].kind else {
            return Err(invalid_manifest());
        };
        let manifest = validate_manifest(descriptor, *bytes, &mut budget, cancellation)?;
        check_cancellation(cancellation)?;

        let output = ToolOutput::success(json!({
            "source": arguments.source,
            "skill": arguments.skill,
            "destination": arguments.destination(),
            "entries": budget.entries,
            "total_bytes": budget.total_bytes,
        }));
        ensure_output(&output)?;

        let mut stage = create_stage(&self.root, &arguments.skill, &mut budget, cancellation)?;
        let build_result = populate_stage(
            stage.skill_root.as_fd(),
            &entries,
            &manifest,
            &mut budget,
            cancellation,
        );
        if let Err(error) = build_result {
            stage.cleanup();
            return Err(error);
        }
        check_cancellation(cancellation).inspect_err(|_| stage.cleanup())?;
        sync_precommit(&stage.skill_root, &mut budget, cancellation)?;
        check_cancellation(cancellation).inspect_err(|_| stage.cleanup())?;
        sync_precommit(&stage.retained_root, &mut budget, cancellation)?;
        check_cancellation(cancellation).inspect_err(|_| stage.cleanup())?;
        stage.publish(&mut budget)?;
        Ok(output)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn scan_directory(
    directory: BorrowedFd<'_>,
    prefix: &str,
    depth: usize,
    entries: &mut Vec<PlannedEntry>,
    budget: &mut Budget,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    if depth > MAX_INSTALL_SKILL_PATH_COMPONENTS {
        return Err(resource_limit());
    }
    budget.charge_io()?;
    let mut stream = Dir::read_from(directory).map_err(|_| source_unavailable())?;
    let mut names = Vec::new();
    loop {
        check_cancellation(cancellation)?;
        budget.charge_io()?;
        let Some(entry) = stream.next() else { break };
        let entry = entry.map_err(|_| source_unavailable())?;
        let bytes = entry.file_name().to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        if depth == MAX_INSTALL_SKILL_PATH_COMPONENTS {
            return Err(resource_limit());
        }
        let name = std::str::from_utf8(bytes).map_err(|_| invalid_entry())?;
        if name.is_empty()
            || name.len() > MAX_INSTALL_SKILL_COMPONENT_BYTES
            || name.chars().any(forbidden_character)
        {
            return Err(invalid_entry());
        }
        budget.admit_entry(name.len())?;
        names.push(name.to_owned());
    }
    names.sort_unstable();
    for name in names {
        check_cancellation(cancellation)?;
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if path.len() > MAX_INSTALL_SKILL_PATH_BYTES {
            return Err(resource_limit());
        }
        budget.charge_io()?;
        let descriptor = rustix::fs::openat(
            directory,
            &name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(map_source_open_error)?;
        budget.charge_io()?;
        let metadata = rustix::fs::fstat(&descriptor).map_err(|_| source_unavailable())?;
        let file_type = FileType::from_raw_mode(metadata.st_mode);
        if file_type.is_dir() {
            entries.push(PlannedEntry {
                path: path.clone(),
                kind: PlannedEntryKind::Directory,
            });
            scan_directory(
                descriptor.as_fd(),
                &path,
                depth + 1,
                entries,
                budget,
                cancellation,
            )?;
        } else if file_type.is_file() {
            let bytes = usize::try_from(metadata.st_size).map_err(|_| resource_limit())?;
            budget.admit_file(bytes)?;
            entries.push(PlannedEntry {
                path,
                kind: PlannedEntryKind::File { descriptor, bytes },
            });
        } else {
            return Err(path_rejected());
        }
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_manifest(
    descriptor: &OwnedFd,
    expected: usize,
    budget: &mut Budget,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, ToolError> {
    budget.charge_io()?;
    rustix::fs::seek(descriptor, SeekFrom::Start(0)).map_err(|_| source_changed())?;
    let mut content = Vec::new();
    content
        .try_reserve_exact(expected)
        .map_err(|_| resource_limit())?;
    let mut buffer = vec![0_u8; MAX_INSTALL_SKILL_CHUNK_BYTES.min(expected.max(1))];
    while content.len() < expected {
        check_cancellation(cancellation)?;
        let count = read_with_budget(descriptor, &mut buffer, budget, cancellation)?;
        if count == 0 {
            return Err(source_changed());
        }
        content.extend_from_slice(&buffer[..count]);
        if content.len() > expected {
            return Err(source_changed());
        }
    }
    if read_with_budget(descriptor, &mut buffer[..1], budget, cancellation)? != 0 {
        return Err(source_changed());
    }
    std::str::from_utf8(&content).map_err(|_| invalid_manifest())?;
    Ok(content)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn populate_stage(
    root: BorrowedFd<'_>,
    entries: &[PlannedEntry],
    manifest: &[u8],
    budget: &mut Budget,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    for entry in entries {
        check_cancellation(cancellation)?;
        let (parent_path, name) = split_parent(&entry.path);
        let parent = open_relative_directory(root, parent_path, budget, cancellation)?;
        match &entry.kind {
            PlannedEntryKind::Directory => {
                budget.charge_io()?;
                rustix::fs::mkdirat(parent.as_fd(), name, Mode::from_raw_mode(0o700))
                    .map_err(|_| write_failed())?;
            }
            PlannedEntryKind::File { descriptor, bytes } => {
                budget.charge_io()?;
                let output = rustix::fs::openat(
                    parent.as_fd(),
                    name,
                    OFlags::WRONLY
                        | OFlags::CREATE
                        | OFlags::EXCL
                        | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC
                        | OFlags::NONBLOCK,
                    Mode::from_raw_mode(0o600),
                )
                .map_err(|_| write_failed())?;
                if entry.path == "SKILL.md" {
                    write_bytes(&output, manifest, budget, cancellation)?;
                } else {
                    copy_exact(descriptor, &output, *bytes, budget, cancellation)?;
                }
                sync_precommit(&output, budget, cancellation)?;
            }
        }
        sync_precommit(&parent, budget, cancellation)?;
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_bytes(
    destination: &OwnedFd,
    content: &[u8],
    budget: &mut Budget,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    let mut offset = 0_usize;
    while offset < content.len() {
        check_cancellation(cancellation)?;
        let end = offset
            .checked_add(MAX_INSTALL_SKILL_CHUNK_BYTES)
            .map_or(content.len(), |end| end.min(content.len()));
        budget.charge_io()?;
        match rustix::io::write(destination, &content[offset..end]) {
            Ok(0) => return Err(write_failed()),
            Ok(count) if count <= end - offset => offset += count,
            Err(error) if error == rustix::io::Errno::INTR => {}
            Ok(_) | Err(_) => return Err(write_failed()),
        }
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn copy_exact(
    source: &OwnedFd,
    destination: &OwnedFd,
    expected: usize,
    budget: &mut Budget,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    budget.charge_io()?;
    rustix::fs::seek(source, SeekFrom::Start(0)).map_err(|_| source_changed())?;
    let mut copied = 0_usize;
    let mut buffer = vec![0_u8; MAX_INSTALL_SKILL_CHUNK_BYTES.min(expected.max(1))];
    while copied < expected {
        check_cancellation(cancellation)?;
        let read = read_with_budget(source, &mut buffer, budget, cancellation)?;
        if read == 0
            || copied
                .checked_add(read)
                .is_none_or(|total| total > expected)
        {
            return Err(source_changed());
        }
        let mut written = 0_usize;
        while written < read {
            check_cancellation(cancellation)?;
            budget.charge_io()?;
            let count = match rustix::io::write(destination, &buffer[written..read]) {
                Ok(count) => count,
                Err(error) if error == rustix::io::Errno::INTR => continue,
                Err(_) => return Err(write_failed()),
            };
            if count == 0 || count > read - written {
                return Err(write_failed());
            }
            written += count;
        }
        copied += read;
    }
    if read_with_budget(source, &mut buffer[..1], budget, cancellation)? != 0 {
        return Err(source_changed());
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_with_budget(
    source: &OwnedFd,
    buffer: &mut [u8],
    budget: &mut Budget,
    cancellation: &CancellationToken,
) -> Result<usize, ToolError> {
    loop {
        check_cancellation(cancellation)?;
        budget.charge_io()?;
        match rustix::io::read(source, &mut *buffer) {
            Ok(count) if count <= buffer.len() => return Ok(count),
            Err(error) if error == rustix::io::Errno::INTR => {}
            Ok(_) | Err(_) => return Err(source_changed()),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_precommit(
    descriptor: &OwnedFd,
    budget: &mut Budget,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    loop {
        check_cancellation(cancellation)?;
        budget.charge_io()?;
        match rustix::fs::fsync(descriptor) {
            Ok(()) => return Ok(()),
            Err(error) if error == rustix::io::Errno::INTR => {}
            Err(_) => return Err(write_failed()),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_postcommit(descriptor: BorrowedFd<'_>) -> Result<(), ToolError> {
    for _ in 0..MAX_POSTCOMMIT_SYNC_ATTEMPTS {
        match rustix::fs::fsync(descriptor) {
            Ok(()) => return Ok(()),
            Err(error) if error == rustix::io::Errno::INTR => {}
            Err(_) => return Err(commit_ambiguous()),
        }
    }
    Err(commit_ambiguous())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct Stage {
    workspace: OwnedFd,
    parent: OwnedFd,
    parent_identity: FileIdentity,
    name: String,
    retained_root: OwnedFd,
    retained_identity: FileIdentity,
    skill_root: OwnedFd,
    destination: String,
    layout: DestinationLayout,
    published: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    device: i128,
    inode: i128,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DestinationLayout {
    ExistingSkills,
    MissingSkills,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Stage {
    fn publish(&mut self, budget: &mut Budget) -> Result<(), ToolError> {
        self.verify_prepublication(budget)?;
        let reserved_operations = match self.layout {
            DestinationLayout::ExistingSkills => 3 + MAX_POSTCOMMIT_SYNC_ATTEMPTS,
            DestinationLayout::MissingSkills => 2 + MAX_POSTCOMMIT_SYNC_ATTEMPTS,
        };
        for _ in 0..reserved_operations {
            budget.charge_io()?;
        }
        let outcome = rustix::fs::renameat_with(
            self.parent.as_fd(),
            &self.name,
            self.parent.as_fd(),
            &self.destination,
            RenameFlags::NOREPLACE,
        );
        match outcome {
            Ok(()) => self.published = true,
            Err(error) if error == rustix::io::Errno::EXIST => return Err(destination_exists()),
            Err(error)
                if error == rustix::io::Errno::NOTSUP
                    || error == rustix::io::Errno::OPNOTSUPP
                    || error == rustix::io::Errno::INVAL =>
            {
                return Err(unsupported_filesystem());
            }
            Err(_) => return Err(commit_ambiguous()),
        }
        self.verify_postpublication()?;
        sync_postcommit(self.parent.as_fd())?;
        Ok(())
    }

    fn verify_prepublication(&self, budget: &mut Budget) -> Result<(), ToolError> {
        budget.charge_io()?;
        let staged = rustix::fs::statat(self.parent.as_fd(), &self.name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| destination_changed())?;
        if file_identity(&staged) != self.retained_identity {
            return Err(destination_changed());
        }

        match self.layout {
            DestinationLayout::ExistingSkills => {
                budget.charge_io()?;
                let current =
                    rustix::fs::statat(self.workspace.as_fd(), "skills", AtFlags::SYMLINK_NOFOLLOW)
                        .map_err(|_| destination_changed())?;
                if file_identity(&current) != self.parent_identity {
                    return Err(destination_changed());
                }
                ensure_absent(self.parent.as_fd(), &self.destination, budget)
            }
            DestinationLayout::MissingSkills => {
                budget.charge_io()?;
                match rustix::fs::statat(
                    self.workspace.as_fd(),
                    "skills",
                    AtFlags::SYMLINK_NOFOLLOW,
                ) {
                    Err(error) if error == rustix::io::Errno::NOENT => Ok(()),
                    Ok(_) => Err(destination_exists()),
                    Err(_) => Err(destination_unavailable()),
                }
            }
        }
    }

    fn verify_postpublication(&self) -> Result<(), ToolError> {
        let current_skills =
            rustix::fs::statat(self.workspace.as_fd(), "skills", AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|_| commit_ambiguous())?;
        match self.layout {
            DestinationLayout::ExistingSkills => {
                if file_identity(&current_skills) != self.parent_identity {
                    return Err(commit_ambiguous());
                }
                let published = rustix::fs::statat(
                    self.parent.as_fd(),
                    &self.destination,
                    AtFlags::SYMLINK_NOFOLLOW,
                )
                .map_err(|_| commit_ambiguous())?;
                if file_identity(&published) != self.retained_identity {
                    return Err(commit_ambiguous());
                }
            }
            DestinationLayout::MissingSkills => {
                if file_identity(&current_skills) != self.retained_identity {
                    return Err(commit_ambiguous());
                }
            }
        }
        Ok(())
    }

    fn cleanup(&mut self) {
        if !self.published {
            let _ = remove_owned_tree(self.parent.as_fd(), &self.name, self.retained_root.as_fd());
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for Stage {
    fn drop(&mut self) {
        self.cleanup();
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn create_stage(
    root: &OwnedFd,
    skill: &str,
    budget: &mut Budget,
    cancellation: &CancellationToken,
) -> Result<Stage, ToolError> {
    budget.charge_io()?;
    let skills = match rustix::fs::openat(root, "skills", directory_flags(), Mode::empty()) {
        Ok(skills) => Some(skills),
        Err(error) if error == rustix::io::Errno::NOENT => None,
        Err(error) => return Err(map_destination_open_error(error)),
    };
    if let Some(skills) = skills {
        ensure_absent(skills.as_fd(), skill, budget)?;
        let parent_identity = descriptor_identity(skills.as_fd(), budget)?;
        let workspace = clone_descriptor(root, budget)?;
        budget.charge_io()?;
        let (name, stage_root, retained_identity) =
            create_stage_directory(skills.as_fd(), budget, cancellation)?;
        let Ok(skill_root) = stage_root.try_clone() else {
            let _ = remove_owned_tree(skills.as_fd(), &name, stage_root.as_fd());
            return Err(destination_unavailable());
        };
        Ok(Stage {
            workspace,
            parent: skills,
            parent_identity,
            name,
            retained_root: stage_root,
            retained_identity,
            skill_root,
            destination: skill.to_owned(),
            layout: DestinationLayout::ExistingSkills,
            published: false,
        })
    } else {
        let parent_identity = descriptor_identity(root.as_fd(), budget)?;
        let workspace = clone_descriptor(root, budget)?;
        let parent = clone_descriptor(root, budget)?;
        budget.charge_io()?;
        budget.charge_io()?;
        let (name, stage_root, retained_identity) =
            create_stage_directory(root.as_fd(), budget, cancellation)?;
        if let Err(error) =
            rustix::fs::mkdirat(stage_root.as_fd(), skill, Mode::from_raw_mode(0o700))
        {
            let _ = remove_owned_tree(root.as_fd(), &name, stage_root.as_fd());
            return Err(map_destination_write_error(error));
        }
        let skill_root =
            match rustix::fs::openat(stage_root.as_fd(), skill, directory_flags(), Mode::empty()) {
                Ok(descriptor) => descriptor,
                Err(error) => {
                    let _ = remove_owned_tree(root.as_fd(), &name, stage_root.as_fd());
                    return Err(map_destination_write_error(error));
                }
            };
        Ok(Stage {
            workspace,
            parent,
            parent_identity,
            name,
            retained_root: stage_root,
            retained_identity,
            skill_root,
            destination: "skills".to_owned(),
            layout: DestinationLayout::MissingSkills,
            published: false,
        })
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn create_stage_directory(
    parent: BorrowedFd<'_>,
    budget: &mut Budget,
    cancellation: &CancellationToken,
) -> Result<(String, OwnedFd, FileIdentity), ToolError> {
    for _ in 0..MAX_INSTALL_SKILL_STAGE_ATTEMPTS {
        check_cancellation(cancellation)?;
        let name = random_stage_name(budget)?;
        for _ in 0..4 {
            budget.charge_io()?;
        }
        match rustix::fs::mkdirat(parent, &name, Mode::from_raw_mode(0o700)) {
            Ok(()) => {
                let observed = rustix::fs::statat(parent, &name, AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(map_destination_write_error)?;
                let identity = file_identity(&observed);
                let descriptor =
                    rustix::fs::openat(parent, &name, directory_flags(), Mode::empty())
                        .map_err(map_destination_write_error)?;
                let retained =
                    rustix::fs::fstat(&descriptor).map_err(|_| destination_unavailable())?;
                if file_identity(&retained) != identity
                    || !FileType::from_raw_mode(retained.st_mode).is_dir()
                {
                    return Err(destination_changed());
                }
                return Ok((name, descriptor, identity));
            }
            Err(error) if error == rustix::io::Errno::EXIST => {}
            Err(error) => return Err(map_destination_write_error(error)),
        }
    }
    Err(resource_limit())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn random_stage_name(budget: &mut Budget) -> Result<String, ToolError> {
    let mut bytes = [0_u8; 16];
    let mut filled = 0_usize;
    while filled < bytes.len() {
        budget.charge_io()?;
        let count = match entropy_read(&mut bytes[filled..]) {
            Ok(count) => count,
            Err(error) if error == rustix::io::Errno::INTR => continue,
            Err(_) => return Err(destination_unavailable()),
        };
        if count == 0 {
            return Err(destination_unavailable());
        }
        filled = filled.checked_add(count).ok_or_else(resource_limit)?;
    }
    let mut name = String::with_capacity(STAGE_PREFIX.len() + bytes.len() * 2);
    name.push_str(STAGE_PREFIX);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut name, "{byte:02x}").map_err(|_| resource_limit())?;
    }
    Ok(name)
}

#[cfg(target_os = "linux")]
fn entropy_read(buffer: &mut [u8]) -> Result<usize, rustix::io::Errno> {
    rustix::rand::getrandom(buffer, rustix::rand::GetRandomFlags::NONBLOCK)
}

#[cfg(target_os = "macos")]
fn entropy_read(buffer: &mut [u8]) -> Result<usize, rustix::io::Errno> {
    let requested = buffer.len();
    getrandom::fill(buffer).map_or_else(
        |error| {
            Err(error.raw_os_error().map_or(rustix::io::Errno::IO, |raw| {
                rustix::io::Errno::from_raw_os_error(raw)
            }))
        },
        |()| Ok(requested),
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn open_relative_directory(
    root: BorrowedFd<'_>,
    path: &str,
    budget: &mut Budget,
    cancellation: &CancellationToken,
) -> Result<OwnedFd, ToolError> {
    budget.charge_io()?;
    let mut current = root
        .try_clone_to_owned()
        .map_err(|_| source_unavailable())?;
    if path.is_empty() {
        return Ok(current);
    }
    for component in path.split('/') {
        check_cancellation(cancellation)?;
        budget.charge_io()?;
        current = rustix::fs::openat(current.as_fd(), component, directory_flags(), Mode::empty())
            .map_err(map_source_open_error)?;
    }
    Ok(current)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn clone_descriptor(descriptor: &OwnedFd, budget: &mut Budget) -> Result<OwnedFd, ToolError> {
    budget.charge_io()?;
    descriptor
        .try_clone()
        .map_err(|_| destination_unavailable())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn split_parent(path: &str) -> (&str, &str) {
    path.rsplit_once('/')
        .map_or(("", path), |(parent, name)| (parent, name))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ensure_absent(parent: BorrowedFd<'_>, name: &str, budget: &mut Budget) -> Result<(), ToolError> {
    budget.charge_io()?;
    match rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Err(error) if error == rustix::io::Errno::NOENT => Ok(()),
        Ok(_) => Err(destination_exists()),
        Err(_) => Err(destination_unavailable()),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn descriptor_identity(
    descriptor: BorrowedFd<'_>,
    budget: &mut Budget,
) -> Result<FileIdentity, ToolError> {
    budget.charge_io()?;
    rustix::fs::fstat(descriptor)
        .map(|metadata| file_identity(&metadata))
        .map_err(|_| destination_unavailable())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn file_identity(metadata: &rustix::fs::Stat) -> FileIdentity {
    FileIdentity {
        device: i128::from(metadata.st_dev),
        inode: i128::from(metadata.st_ino),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct CleanupBudget {
    operations: usize,
    entries: usize,
    name_bytes: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl CleanupBudget {
    const fn new() -> Self {
        Self {
            operations: 0,
            entries: 0,
            name_bytes: 0,
        }
    }

    fn charge_operation(&mut self) -> Result<(), ()> {
        self.operations = self.operations.checked_add(1).ok_or(())?;
        if self.operations > MAX_INSTALL_SKILL_IO_ATTEMPTS {
            Err(())
        } else {
            Ok(())
        }
    }

    fn admit_entry(&mut self, bytes: usize) -> Result<(), ()> {
        self.entries = self.entries.checked_add(1).ok_or(())?;
        self.name_bytes = self.name_bytes.checked_add(bytes).ok_or(())?;
        if self.entries > MAX_INSTALL_SKILL_ENTRIES + 1
            || self.name_bytes > MAX_INSTALL_SKILL_ENTRY_NAME_BYTES + MAX_INSTALL_SKILL_NAME_BYTES
        {
            Err(())
        } else {
            Ok(())
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn remove_tree(
    parent: BorrowedFd<'_>,
    name: &str,
    depth: usize,
    budget: &mut CleanupBudget,
) -> Result<(), ()> {
    if depth > MAX_INSTALL_SKILL_PATH_COMPONENTS + 1 {
        return Err(());
    }
    budget.charge_operation()?;
    let directory =
        rustix::fs::openat(parent, name, directory_flags(), Mode::empty()).map_err(|_| ())?;
    budget.charge_operation()?;
    let mut stream = Dir::read_from(directory.as_fd()).map_err(|_| ())?;
    let mut names = Vec::new();
    for entry in stream.by_ref() {
        budget.charge_operation()?;
        let entry = entry.map_err(|_| ())?;
        let bytes = entry.file_name().to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        budget.admit_entry(bytes.len())?;
        let child = std::str::from_utf8(bytes).map_err(|_| ())?.to_owned();
        names.push((child, entry.file_type().is_dir()));
    }
    drop(stream);
    for (child, directory_entry) in names {
        if directory_entry {
            remove_tree(directory.as_fd(), &child, depth + 1, budget)?;
        } else {
            budget.charge_operation()?;
            rustix::fs::unlinkat(directory.as_fd(), &child, AtFlags::empty()).map_err(|_| ())?;
        }
    }
    budget.charge_operation()?;
    rustix::fs::unlinkat(parent, name, AtFlags::REMOVEDIR).map_err(|_| ())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn remove_owned_tree(
    parent: BorrowedFd<'_>,
    name: &str,
    retained: BorrowedFd<'_>,
) -> Result<(), ()> {
    let mut budget = CleanupBudget::new();
    budget.charge_operation()?;
    let expected = rustix::fs::fstat(retained).map_err(|_| ())?;
    budget.charge_operation()?;
    let observed = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|_| ())?;
    if expected.st_dev != observed.st_dev || expected.st_ino != observed.st_ino {
        return Err(());
    }
    remove_tree(parent, name, 0, &mut budget)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
const fn directory_flags() -> OFlags {
    OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC)
        .union(OFlags::NONBLOCK)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_root_open_error(error: rustix::io::Errno) -> InstallSkillToolOpenError {
    let kind = if error == rustix::io::Errno::LOOP
        || error == rustix::io::Errno::MLINK
        || error == rustix::io::Errno::NOTDIR
    {
        InstallSkillToolOpenErrorKind::InvalidFileType
    } else {
        InstallSkillToolOpenErrorKind::Unavailable
    };
    InstallSkillToolOpenError::new(kind)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_source_open_error(error: rustix::io::Errno) -> ToolError {
    if error == rustix::io::Errno::NOENT {
        source_not_found()
    } else if error == rustix::io::Errno::LOOP
        || error == rustix::io::Errno::MLINK
        || error == rustix::io::Errno::NOTDIR
        || error == rustix::io::Errno::NXIO
    {
        path_rejected()
    } else if error == rustix::io::Errno::ACCESS || error == rustix::io::Errno::PERM {
        permission_denied()
    } else {
        source_unavailable()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_destination_open_error(error: rustix::io::Errno) -> ToolError {
    if error == rustix::io::Errno::LOOP
        || error == rustix::io::Errno::MLINK
        || error == rustix::io::Errno::NOTDIR
    {
        path_rejected()
    } else if error == rustix::io::Errno::ACCESS || error == rustix::io::Errno::PERM {
        permission_denied()
    } else {
        destination_unavailable()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_destination_write_error(error: rustix::io::Errno) -> ToolError {
    if error == rustix::io::Errno::ACCESS || error == rustix::io::Errno::PERM {
        permission_denied()
    } else {
        write_failed()
    }
}

fn check_cancellation(cancellation: &CancellationToken) -> Result<(), ToolError> {
    if cancellation.is_cancelled() {
        Err(cancelled())
    } else {
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ensure_output(output: &ToolOutput) -> Result<(), ToolError> {
    serde_json::to_vec(output)
        .ok()
        .filter(|bytes| bytes.len() <= MAX_INSTALL_SKILL_SERIALIZED_RESULT_BYTES)
        .map(|_| ())
        .ok_or_else(resource_limit)
}

fn invalid_arguments() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "install_skill_invalid_arguments",
        "install_skill arguments are invalid",
        false,
    )
}
fn invalid_source() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "install_skill_invalid_source",
        "install_skill source is invalid",
        false,
    )
}
fn invalid_skill() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "install_skill_invalid_skill",
        "install_skill skill name is invalid",
        false,
    )
}
fn overlap() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "install_skill_overlap",
        "install_skill source overlaps its managed destination",
        false,
    )
}
fn resource_limit() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "install_skill_resource_limit",
        "install_skill resource limit was exceeded",
        false,
    )
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn invalid_entry() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "install_skill_invalid_entry",
        "install_skill source contains an invalid entry name",
        false,
    )
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn invalid_manifest() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "install_skill_invalid_manifest",
        "install_skill source requires a regular UTF-8 SKILL.md",
        false,
    )
}
fn cancelled() -> ToolError {
    ToolError::new(
        ToolErrorKind::Cancelled,
        "install_skill_cancelled",
        "install_skill execution was cancelled",
        false,
    )
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported_platform() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "install_skill_unsupported_platform",
        "native install_skill is unsupported on this platform",
        false,
    )
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn source_not_found() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "install_skill_source_not_found",
        "install_skill source is unavailable",
        false,
    )
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn source_unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "install_skill_source_unavailable",
        "install_skill source is unavailable",
        true,
    )
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn source_changed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "install_skill_source_changed",
        "install_skill source changed during installation",
        true,
    )
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn path_rejected() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "install_skill_path_rejected",
        "install_skill path is not confined",
        false,
    )
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn permission_denied() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "install_skill_permission_denied",
        "install_skill filesystem access was denied",
        false,
    )
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn destination_exists() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "install_skill_destination_exists",
        "install_skill destination already exists",
        false,
    )
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn destination_unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "install_skill_destination_unavailable",
        "install_skill destination is unavailable",
        true,
    )
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn destination_changed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "install_skill_destination_changed",
        "install_skill destination changed before publication",
        true,
    )
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_failed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "install_skill_write_failed",
        "install_skill staged copy failed",
        true,
    )
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unsupported_filesystem() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "install_skill_unsupported_filesystem",
        "atomic no-replace skill publication is unavailable",
        false,
    )
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn commit_ambiguous() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "install_skill_commit_ambiguous",
        "install_skill publication status is uncertain",
        false,
    )
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use machine_god_core::CancellationToken;

    use super::{Budget, InstallSkillTool, create_stage, destination_changed, directory_flags};
    use rustix::fd::AsFd;
    use rustix::fs::{Mode, RenameFlags};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TemporaryDirectory(PathBuf);

    impl TemporaryDirectory {
        fn new() -> Self {
            for _ in 0..1_000 {
                let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "mg-install-skill-private-{}-{id}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("create temporary directory: {error}"),
                }
            }
            panic!("allocate temporary directory")
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn relocated_existing_skills_is_rejected_before_publication() {
        let workspace = TemporaryDirectory::new();
        fs::create_dir(workspace.path().join("skills")).unwrap();
        let tool = InstallSkillTool::open(workspace.path()).unwrap();
        let mut budget = Budget::new();
        let mut stage =
            create_stage(&tool.root, "rust", &mut budget, &CancellationToken::new()).unwrap();

        fs::rename(
            workspace.path().join("skills"),
            workspace.path().join("displaced"),
        )
        .unwrap();
        fs::create_dir(workspace.path().join("skills")).unwrap();

        let error = stage.publish(&mut budget).unwrap_err();
        assert_eq!(error.code, destination_changed().code);
        assert!(!workspace.path().join("skills/rust").exists());
    }

    #[test]
    fn relocation_in_the_publish_window_is_postcommit_ambiguity_not_success() {
        let workspace = TemporaryDirectory::new();
        fs::create_dir(workspace.path().join("skills")).unwrap();
        let tool = InstallSkillTool::open(workspace.path()).unwrap();
        let mut budget = Budget::new();
        let mut stage =
            create_stage(&tool.root, "rust", &mut budget, &CancellationToken::new()).unwrap();
        stage.verify_prepublication(&mut budget).unwrap();

        fs::rename(
            workspace.path().join("skills"),
            workspace.path().join("displaced"),
        )
        .unwrap();
        fs::create_dir(workspace.path().join("skills")).unwrap();
        rustix::fs::renameat_with(
            stage.parent.as_fd(),
            &stage.name,
            stage.parent.as_fd(),
            &stage.destination,
            RenameFlags::NOREPLACE,
        )
        .unwrap();
        stage.published = true;

        let error = stage.verify_postpublication().unwrap_err();
        assert_eq!(error.code, "install_skill_commit_ambiguous");
        assert!(!workspace.path().join("skills/rust").exists());
        assert!(workspace.path().join("displaced/rust").exists());
    }

    #[test]
    fn cleanup_preserves_a_replacement_stage_name() {
        let workspace = TemporaryDirectory::new();
        fs::create_dir(workspace.path().join("skills")).unwrap();
        let tool = InstallSkillTool::open(workspace.path()).unwrap();
        let mut budget = Budget::new();
        let mut stage =
            create_stage(&tool.root, "rust", &mut budget, &CancellationToken::new()).unwrap();
        let original = workspace.path().join("skills").join(&stage.name);
        let displaced = workspace.path().join("skills/displaced-stage");
        fs::rename(&original, &displaced).unwrap();
        fs::create_dir(&original).unwrap();
        fs::write(original.join("sentinel"), b"replacement").unwrap();

        stage.cleanup();
        assert_eq!(fs::read(original.join("sentinel")).unwrap(), b"replacement");
        assert!(displaced.exists());
    }

    #[test]
    fn cleanup_stops_at_its_entry_bound() {
        let workspace = TemporaryDirectory::new();
        fs::create_dir(workspace.path().join("skills")).unwrap();
        let tool = InstallSkillTool::open(workspace.path()).unwrap();
        let mut budget = Budget::new();
        let mut stage =
            create_stage(&tool.root, "rust", &mut budget, &CancellationToken::new()).unwrap();
        let stage_path = workspace.path().join("skills").join(&stage.name);
        for index in 0..300 {
            fs::write(stage_path.join(format!("injected-{index:03}")), b"").unwrap();
        }
        stage.cleanup();
        assert!(stage_path.exists());

        let descriptor = rustix::fs::open(
            workspace.path().join("skills"),
            directory_flags(),
            Mode::empty(),
        )
        .unwrap();
        assert!(rustix::fs::fstat(descriptor).is_ok());
    }
}
