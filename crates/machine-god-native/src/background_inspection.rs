use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use machine_god_core::BoxFuture;

use crate::NativeEnvironment;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::state_environment::{ProcessStateEnvironmentReader, capture_state_environment};

/// Maximum number of persisted background records returned by one listing.
pub const MAX_BACKGROUND_RECORDS: usize = 100;
/// Maximum number of non-dot entries processed by one listing.
pub const MAX_BACKGROUND_DIRECTORY_ENTRIES: usize = 1_024;
/// Maximum encoded size of one persisted background record.
pub const MAX_BACKGROUND_RECORD_BYTES: usize = 64 * 1_024;
/// Maximum aggregate record bytes accepted by one listing.
pub const MAX_BACKGROUND_TOTAL_RECORD_BYTES: usize = 8 * 1_024 * 1_024;
/// Maximum encoded workspace or working-directory path length.
pub const MAX_BACKGROUND_PATH_BYTES: usize = 4_096;
/// Maximum command length in one persisted record.
pub const MAX_BACKGROUND_COMMAND_BYTES: usize = 32 * 1_024;
/// Maximum command preview length in one list row.
pub const MAX_BACKGROUND_COMMAND_PREVIEW_BYTES: usize = 256;
/// Maximum optional server URL length.
pub const MAX_BACKGROUND_SERVER_URL_BYTES: usize = 2_048;
/// Maximum optional diagnostic length.
pub const MAX_BACKGROUND_DIAGNOSTIC_BYTES: usize = 4_096;
/// Maximum JSON container depth accepted by the persisted schema.
pub const MAX_BACKGROUND_JSON_DEPTH: usize = 4;
/// Maximum JSON values accepted by the persisted schema.
pub const MAX_BACKGROUND_JSON_NODES: usize = 64;

#[cfg(any(target_os = "linux", target_os = "macos"))]
const WORKSPACE_DIGEST_DOMAIN: &[u8] = b"machine-god:background-workspace:v1:";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const RECORD_DIGEST_DOMAIN: &[u8] = b"machine-god:background-record:v1:";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const BACKGROUND_DIRECTORY: &str = "background-v1";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const RECORD_PREFIX: &[u8] = b"record-";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const RECORD_SUFFIX: &[u8] = b".json";

/// Bounded native background-history query.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeBackgroundQuery {
    /// List validated records in authoritative newest-first order.
    List,
    /// Return the authoritative latest record.
    Last,
    /// Return exactly one numeric record.
    Id(u64),
}

/// Recorded background history state. This is not a liveness assertion.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NativeBackgroundState {
    Running,
    Exited,
    Failed,
    Stopped,
    Dead,
    Stale,
}

impl NativeBackgroundState {
    /// Returns the stable schema spelling of this state.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Exited => "exited",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
            Self::Dead => "dead",
            Self::Stale => "stale",
        }
    }
}

/// Bounded summary of one validated persisted record.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeBackgroundRecordSummary {
    id: u64,
    state: NativeBackgroundState,
    updated_at_ms: u64,
    command_preview: String,
    preview_truncated: bool,
}

impl NativeBackgroundRecordSummary {
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    #[must_use]
    pub const fn state(&self) -> NativeBackgroundState {
        self.state
    }

    #[must_use]
    pub const fn updated_at_ms(&self) -> u64 {
        self.updated_at_ms
    }

    #[must_use]
    pub fn command_preview(&self) -> &str {
        &self.command_preview
    }

    #[must_use]
    pub const fn preview_truncated(&self) -> bool {
        self.preview_truncated
    }
}

impl fmt::Debug for NativeBackgroundRecordSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeBackgroundRecordSummary")
            .field("id", &self.id)
            .field("state", &self.state)
            .field("updated_at_ms", &self.updated_at_ms)
            .field("preview_truncated", &self.preview_truncated)
            .finish_non_exhaustive()
    }
}

/// Bounded result of one persisted-history scan.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeBackgroundList {
    records: Vec<NativeBackgroundRecordSummary>,
    truncated: bool,
}

impl NativeBackgroundList {
    #[must_use]
    pub fn records(&self) -> &[NativeBackgroundRecordSummary] {
        &self.records
    }

    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }
}

impl fmt::Debug for NativeBackgroundList {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeBackgroundList")
            .field("record_count", &self.records.len())
            .field("truncated", &self.truncated)
            .finish()
    }
}

/// Full bounded detail from one validated persisted record.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeBackgroundDetail {
    id: u64,
    state: NativeBackgroundState,
    started_at_ms: u64,
    updated_at_ms: u64,
    pid: Option<u32>,
    command: String,
    cwd: String,
    exit_code: Option<i32>,
    server_url: Option<String>,
    diagnostic: Option<String>,
}

impl NativeBackgroundDetail {
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    #[must_use]
    pub const fn state(&self) -> NativeBackgroundState {
        self.state
    }

    #[must_use]
    pub const fn started_at_ms(&self) -> u64 {
        self.started_at_ms
    }

    #[must_use]
    pub const fn updated_at_ms(&self) -> u64 {
        self.updated_at_ms
    }

    #[must_use]
    pub const fn pid(&self) -> Option<u32> {
        self.pid
    }

    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }

    #[must_use]
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    #[must_use]
    pub const fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    #[must_use]
    pub fn server_url(&self) -> Option<&str> {
        self.server_url.as_deref()
    }

    #[must_use]
    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn summary(&self) -> NativeBackgroundRecordSummary {
        let boundary = floor_char_boundary(&self.command, MAX_BACKGROUND_COMMAND_PREVIEW_BYTES);
        NativeBackgroundRecordSummary {
            id: self.id,
            state: self.state,
            updated_at_ms: self.updated_at_ms,
            command_preview: self.command[..boundary].to_owned(),
            preview_truncated: boundary < self.command.len(),
        }
    }
}

impl fmt::Debug for NativeBackgroundDetail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeBackgroundDetail")
            .field("id", &self.id)
            .field("state", &self.state)
            .field("started_at_ms", &self.started_at_ms)
            .field("updated_at_ms", &self.updated_at_ms)
            .field("pid", &self.pid)
            .field("exit_code", &self.exit_code)
            .field("has_server_url", &self.server_url.is_some())
            .field("has_diagnostic", &self.diagnostic.is_some())
            .finish_non_exhaustive()
    }
}

/// Result of a native background-history inspection.
#[derive(Clone, Eq, PartialEq)]
pub enum NativeBackgroundInspection {
    List(NativeBackgroundList),
    Detail(NativeBackgroundDetail),
}

impl fmt::Debug for NativeBackgroundInspection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::List(list) => formatter.debug_tuple("List").field(list).finish(),
            Self::Detail(detail) => formatter.debug_tuple("Detail").field(detail).finish(),
        }
    }
}

/// Stable category for native background-inspection failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeBackgroundInspectionErrorKind {
    NotFound,
    Corrupt,
    ResourceLimit,
    Unavailable,
    UnsupportedPlatform,
}

impl NativeBackgroundInspectionErrorKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Corrupt => "corrupt",
            Self::ResourceLimit => "resource_limit",
            Self::Unavailable => "unavailable",
            Self::UnsupportedPlatform => "unsupported_platform",
        }
    }
}

/// Fixed, redacted native background-inspection failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct NativeBackgroundInspectionError {
    kind: NativeBackgroundInspectionErrorKind,
}

impl NativeBackgroundInspectionError {
    #[must_use]
    pub const fn kind(&self) -> NativeBackgroundInspectionErrorKind {
        self.kind
    }

    const fn new(kind: NativeBackgroundInspectionErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for NativeBackgroundInspectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeBackgroundInspectionError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for NativeBackgroundInspectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            NativeBackgroundInspectionErrorKind::NotFound => {
                "native background record was not found"
            }
            NativeBackgroundInspectionErrorKind::Corrupt => "native background history is corrupt",
            NativeBackgroundInspectionErrorKind::ResourceLimit => {
                "native background inspection reached a resource limit"
            }
            NativeBackgroundInspectionErrorKind::Unavailable => {
                "native background persistence is unavailable"
            }
            NativeBackgroundInspectionErrorKind::UnsupportedPlatform => {
                "native background inspection is unsupported on this platform"
            }
        })
    }
}

impl Error for NativeBackgroundInspectionError {}

/// Inspects persisted background history for an injected workspace and environment.
///
/// Construction is effect-inert. All path validation, canonicalization,
/// hashing, environment interpretation, and filesystem work begins when the
/// returned future is first polled.
#[must_use]
pub fn inspect_native_background(
    environment: NativeEnvironment,
    workspace_root: PathBuf,
    query: NativeBackgroundQuery,
) -> BoxFuture<'static, Result<NativeBackgroundInspection, NativeBackgroundInspectionError>> {
    Box::pin(async move { inspect_native_background_polled(&environment, &workspace_root, query) })
}

/// Captures process inputs and inspects persisted background history on first poll.
#[must_use]
pub fn inspect_process_background(
    query: NativeBackgroundQuery,
) -> BoxFuture<'static, Result<NativeBackgroundInspection, NativeBackgroundInspectionError>> {
    Box::pin(async move {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let mut reader = ProcessStateEnvironmentReader;
            let environment = capture_state_environment(&mut reader);
            let workspace_root = std::env::current_dir().map_err(|_| unavailable())?;
            inspect_native_background_polled(&environment, &workspace_root, query)
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = query;
            Err(NativeBackgroundInspectionError::new(
                NativeBackgroundInspectionErrorKind::UnsupportedPlatform,
            ))
        }
    })
}

fn inspect_native_background_polled(
    environment: &NativeEnvironment,
    workspace_root: &Path,
    query: NativeBackgroundQuery,
) -> Result<NativeBackgroundInspection, NativeBackgroundInspectionError> {
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (environment, workspace_root, query);
        Err(NativeBackgroundInspectionError::new(
            NativeBackgroundInspectionErrorKind::UnsupportedPlatform,
        ))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        supported::inspect(environment, workspace_root, query)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn floor_char_boundary(value: &str, maximum: usize) -> usize {
    if value.len() <= maximum {
        return value.len();
    }
    let mut boundary = maximum;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
const fn unavailable() -> NativeBackgroundInspectionError {
    NativeBackgroundInspectionError::new(NativeBackgroundInspectionErrorKind::Unavailable)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod supported {
    use std::ffi::OsStr;
    use std::io::Read;
    use std::path::{Path, PathBuf};

    use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
    #[cfg(target_os = "linux")]
    use rustix::fs::CWD;
    use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};
    use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
    use serde::{Deserialize, Deserializer};
    use sha2::{Digest, Sha256};

    use super::{
        BACKGROUND_DIRECTORY, MAX_BACKGROUND_COMMAND_BYTES, MAX_BACKGROUND_DIAGNOSTIC_BYTES,
        MAX_BACKGROUND_DIRECTORY_ENTRIES, MAX_BACKGROUND_JSON_DEPTH, MAX_BACKGROUND_JSON_NODES,
        MAX_BACKGROUND_PATH_BYTES, MAX_BACKGROUND_RECORD_BYTES, MAX_BACKGROUND_RECORDS,
        MAX_BACKGROUND_SERVER_URL_BYTES, MAX_BACKGROUND_TOTAL_RECORD_BYTES, NativeBackgroundDetail,
        NativeBackgroundInspection, NativeBackgroundInspectionError,
        NativeBackgroundInspectionErrorKind, NativeBackgroundList, NativeBackgroundQuery,
        NativeBackgroundState, RECORD_DIGEST_DOMAIN, RECORD_PREFIX, RECORD_SUFFIX,
        WORKSPACE_DIGEST_DOMAIN, unavailable,
    };
    use crate::NativeEnvironment;

    const GROUP_OR_OTHER_WRITE: u64 = 0o022;
    const GROUP_OR_OTHER_PERMISSIONS: u64 = 0o077;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct StoredBackgroundRecord {
        version: u32,
        workspace: String,
        id: u64,
        started_at_ms: u64,
        updated_at_ms: u64,
        command: String,
        cwd: String,
        state: NativeBackgroundState,
        #[serde(deserialize_with = "required_option")]
        pid: Option<u32>,
        #[serde(deserialize_with = "required_option")]
        exit_code: Option<i32>,
        #[serde(deserialize_with = "required_option")]
        server_url: Option<String>,
        #[serde(deserialize_with = "required_option")]
        diagnostic: Option<String>,
    }

    fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Option::<T>::deserialize(deserializer)
    }

    pub(super) fn inspect(
        environment: &NativeEnvironment,
        workspace_root: &Path,
        query: NativeBackgroundQuery,
    ) -> Result<NativeBackgroundInspection, NativeBackgroundInspectionError> {
        if !workspace_root.is_absolute() {
            return Err(unavailable());
        }
        let workspace = std::fs::canonicalize(workspace_root).map_err(|_| unavailable())?;
        let workspace = workspace.to_str().ok_or_else(unavailable)?;
        if workspace.len() > MAX_BACKGROUND_PATH_BYTES {
            return Err(NativeBackgroundInspectionError::new(
                NativeBackgroundInspectionErrorKind::ResourceLimit,
            ));
        }

        let hierarchy = open_workspace_hierarchy(environment, workspace)?;
        match (hierarchy, query) {
            (None, NativeBackgroundQuery::List) => {
                Ok(NativeBackgroundInspection::List(NativeBackgroundList {
                    records: Vec::new(),
                    truncated: false,
                }))
            }
            (None, NativeBackgroundQuery::Last | NativeBackgroundQuery::Id(_)) => Err(
                NativeBackgroundInspectionError::new(NativeBackgroundInspectionErrorKind::NotFound),
            ),
            (Some(root), NativeBackgroundQuery::List) => {
                let (listing, _) = list(root.as_fd(), workspace)?;
                Ok(NativeBackgroundInspection::List(listing))
            }
            (Some(root), NativeBackgroundQuery::Last) => {
                let (listing, latest) = list(root.as_fd(), workspace)?;
                if listing.truncated {
                    return Err(NativeBackgroundInspectionError::new(
                        NativeBackgroundInspectionErrorKind::ResourceLimit,
                    ));
                }
                let detail = latest.ok_or_else(|| {
                    NativeBackgroundInspectionError::new(
                        NativeBackgroundInspectionErrorKind::NotFound,
                    )
                })?;
                Ok(NativeBackgroundInspection::Detail(detail))
            }
            (Some(root), NativeBackgroundQuery::Id(id)) => {
                let detail = read_record(root.as_fd(), workspace, id)?.ok_or_else(|| {
                    NativeBackgroundInspectionError::new(
                        NativeBackgroundInspectionErrorKind::NotFound,
                    )
                })?;
                Ok(NativeBackgroundInspection::Detail(detail))
            }
        }
    }

    fn state_base_and_suffix(
        environment: &NativeEnvironment,
    ) -> Result<(PathBuf, &'static [&'static str]), NativeBackgroundInspectionError> {
        if let Some(value) = nonempty(environment.xdg_state_home.as_deref()) {
            let base = validate_state_base(value)?;
            Ok((base, &[crate::STATE_NAMESPACE, BACKGROUND_DIRECTORY]))
        } else if let Some(value) = nonempty(environment.home.as_deref()) {
            let base = validate_state_base(value)?;
            Ok((
                base,
                &[
                    ".local",
                    "state",
                    crate::STATE_NAMESPACE,
                    BACKGROUND_DIRECTORY,
                ],
            ))
        } else {
            Err(unavailable())
        }
    }

    fn nonempty(value: Option<&OsStr>) -> Option<&OsStr> {
        value.filter(|value| !value.is_empty())
    }

    fn validate_state_base(value: &OsStr) -> Result<PathBuf, NativeBackgroundInspectionError> {
        let path = Path::new(value);
        let Some(value) = value.to_str() else {
            return Err(unavailable());
        };
        if !is_canonical_absolute_path(value) {
            return Err(unavailable());
        }
        Ok(path.to_owned())
    }

    fn open_workspace_hierarchy(
        environment: &NativeEnvironment,
        workspace: &str,
    ) -> Result<Option<OwnedFd>, NativeBackgroundInspectionError> {
        let (base, suffix) = state_base_and_suffix(environment)?;
        let Some(mut directory) = open_base(&base)? else {
            return Ok(None);
        };
        validate_directory(&directory, false)?;
        let first_private = suffix
            .iter()
            .position(|component| *component == crate::STATE_NAMESPACE)
            .expect("the fixed background suffix contains the state namespace");
        for (index, component) in suffix.iter().enumerate() {
            let Some(next) = open_child_directory(directory.as_fd(), component)? else {
                return Ok(None);
            };
            validate_directory(&next, index >= first_private)?;
            directory = next;
        }
        let workspace_name = workspace_name(workspace);
        let Some(workspace_directory) = open_child_directory(directory.as_fd(), &workspace_name)?
        else {
            return Ok(None);
        };
        validate_directory(&workspace_directory, true)?;
        Ok(Some(workspace_directory))
    }

    fn open_base(path: &Path) -> Result<Option<OwnedFd>, NativeBackgroundInspectionError> {
        let directory = match open_absolute_directory(path) {
            Ok(directory) => directory,
            Err(error) if error == rustix::io::Errno::NOENT => return Ok(None),
            Err(_) => return Err(unavailable()),
        };
        let metadata = rustix::fs::fstat(&directory).map_err(|_| unavailable())?;
        if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
            return Err(unavailable());
        }
        Ok(Some(directory))
    }

    #[cfg(target_os = "linux")]
    fn open_absolute_directory(path: &Path) -> rustix::io::Result<OwnedFd> {
        rustix::fs::openat2(
            CWD,
            path,
            directory_open_flags(),
            Mode::empty(),
            rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
        )
    }

    #[cfg(target_os = "macos")]
    fn open_absolute_directory(path: &Path) -> rustix::io::Result<OwnedFd> {
        let nofollow_any = OFlags::from_bits_retain(libc::O_NOFOLLOW_ANY as _);
        rustix::fs::open(path, directory_open_flags() | nofollow_any, Mode::empty())
    }

    fn open_child_directory(
        parent: BorrowedFd<'_>,
        name: &str,
    ) -> Result<Option<OwnedFd>, NativeBackgroundInspectionError> {
        let metadata = match rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(metadata) => metadata,
            Err(error) if error == rustix::io::Errno::NOENT => return Ok(None),
            Err(_) => return Err(unavailable()),
        };
        if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
            return Err(unavailable());
        }
        let directory = rustix::fs::openat(
            parent,
            name,
            directory_open_flags() | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| unavailable())?;
        ensure_same_identity(&metadata, &directory)?;
        Ok(Some(directory))
    }

    fn ensure_same_identity(
        path_metadata: &rustix::fs::Stat,
        descriptor: &OwnedFd,
    ) -> Result<(), NativeBackgroundInspectionError> {
        let descriptor_metadata = rustix::fs::fstat(descriptor).map_err(|_| unavailable())?;
        if descriptor_metadata.st_dev != path_metadata.st_dev
            || descriptor_metadata.st_ino != path_metadata.st_ino
            || !FileType::from_raw_mode(descriptor_metadata.st_mode).is_dir()
        {
            return Err(unavailable());
        }
        Ok(())
    }

    fn validate_directory(
        directory: &OwnedFd,
        private: bool,
    ) -> Result<(), NativeBackgroundInspectionError> {
        let metadata = rustix::fs::fstat(directory).map_err(|_| unavailable())?;
        let permissions = u64::from(metadata.st_mode);
        if !FileType::from_raw_mode(metadata.st_mode).is_dir()
            || metadata.st_uid != rustix::process::geteuid().as_raw()
            || permissions & GROUP_OR_OTHER_WRITE != 0
            || (private && permissions & GROUP_OR_OTHER_PERMISSIONS != 0)
        {
            return Err(unavailable());
        }
        #[cfg(target_os = "macos")]
        validate_acl(directory)?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn validate_acl(directory: &OwnedFd) -> Result<(), NativeBackgroundInspectionError> {
        let acl = calcifer_macos_acl::read_acl(directory.as_fd()).map_err(|_| unavailable())?;
        if acl.flags != 0
            || acl.entries.iter().any(|entry| {
                entry.tag != calcifer_macos_acl::TAG_DENY
                    || entry.flags != 0
                    || entry.permissions != calcifer_macos_acl::PERMISSION_DELETE
            })
        {
            return Err(unavailable());
        }
        Ok(())
    }

    fn list(
        root: BorrowedFd<'_>,
        workspace: &str,
    ) -> Result<
        (NativeBackgroundList, Option<NativeBackgroundDetail>),
        NativeBackgroundInspectionError,
    > {
        let duplicate = rustix::fs::openat(
            root,
            ".",
            directory_open_flags() | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| unavailable())?;
        let mut stream = Dir::new(duplicate).map_err(|_| unavailable())?;
        let mut candidates = Vec::new();
        let mut scanned = 0_usize;
        let mut truncated = false;
        loop {
            let Some(entry) = stream.next() else {
                break;
            };
            let entry = entry.map_err(|_| unavailable())?;
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            if scanned == MAX_BACKGROUND_DIRECTORY_ENTRIES {
                truncated = true;
                break;
            }
            scanned += 1;
            if is_record_name(name) {
                candidates.push(
                    std::str::from_utf8(name)
                        .expect("canonical background names are ASCII")
                        .to_owned(),
                );
            }
        }
        candidates.sort_unstable();
        candidates.dedup();

        let mut records = Vec::new();
        let mut latest: Option<NativeBackgroundDetail> = None;
        let mut accepted_bytes = 0_usize;
        for name in candidates {
            if records.len() == MAX_BACKGROUND_RECORDS {
                truncated = true;
                break;
            }
            let remaining = MAX_BACKGROUND_TOTAL_RECORD_BYTES
                .checked_sub(accepted_bytes)
                .expect("accepted background bytes stay within their bound");
            if remaining == 0 {
                truncated = true;
                break;
            }
            let Some((detail, encoded_bytes)) = read_named_record(root, workspace, &name)? else {
                continue;
            };
            if encoded_bytes > remaining {
                truncated = true;
                break;
            }
            accepted_bytes += encoded_bytes;
            let summary = detail.summary();
            if latest.as_ref().is_none_or(|current| {
                (detail.updated_at_ms, detail.id) > (current.updated_at_ms, current.id)
            }) {
                latest = Some(detail);
            }
            records.push(summary);
        }
        records.sort_unstable_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| right.id.cmp(&left.id))
        });
        Ok((NativeBackgroundList { records, truncated }, latest))
    }

    fn read_record(
        root: BorrowedFd<'_>,
        workspace: &str,
        id: u64,
    ) -> Result<Option<NativeBackgroundDetail>, NativeBackgroundInspectionError> {
        let name = record_name(id);
        let Some((detail, _)) = read_named_record(root, workspace, &name)? else {
            return Ok(None);
        };
        if detail.id != id {
            return Err(corrupt());
        }
        Ok(Some(detail))
    }

    fn read_named_record(
        root: BorrowedFd<'_>,
        workspace: &str,
        name: &str,
    ) -> Result<Option<(NativeBackgroundDetail, usize)>, NativeBackgroundInspectionError> {
        let descriptor = match rustix::fs::openat(
            root,
            name,
            record_open_flags() | OFlags::NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(descriptor) => descriptor,
            Err(error) if error == rustix::io::Errno::NOENT => return Ok(None),
            Err(error)
                if is_rejected_type_error(error)
                    || rustix::fs::statat(root, name, AtFlags::SYMLINK_NOFOLLOW).is_ok_and(
                        |metadata| {
                            !FileType::from_raw_mode(metadata.st_mode).is_file()
                                || metadata.st_uid != rustix::process::geteuid().as_raw()
                                || u64::from(metadata.st_mode) & GROUP_OR_OTHER_PERMISSIONS != 0
                        },
                    ) =>
            {
                return Err(corrupt());
            }
            Err(_) => return Err(unavailable()),
        };
        let metadata = rustix::fs::fstat(&descriptor).map_err(|_| unavailable())?;
        if !FileType::from_raw_mode(metadata.st_mode).is_file()
            || metadata.st_uid != rustix::process::geteuid().as_raw()
            || u64::from(metadata.st_mode) & GROUP_OR_OTHER_PERMISSIONS != 0
        {
            return Err(corrupt());
        }
        #[cfg(target_os = "macos")]
        validate_record_acl(&descriptor)?;

        let mut bytes = Vec::with_capacity(MAX_BACKGROUND_RECORD_BYTES + 1);
        let mut file = std::fs::File::from(descriptor);
        file.by_ref()
            .take((MAX_BACKGROUND_RECORD_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| unavailable())?;
        if bytes.len() > MAX_BACKGROUND_RECORD_BYTES {
            return Err(corrupt());
        }
        let detail = decode_record(&bytes, workspace, name)?;
        Ok(Some((detail, bytes.len())))
    }

    #[cfg(target_os = "macos")]
    fn validate_record_acl(descriptor: &OwnedFd) -> Result<(), NativeBackgroundInspectionError> {
        let acl = calcifer_macos_acl::read_acl(descriptor.as_fd()).map_err(|_| unavailable())?;
        if acl.flags != 0
            || acl.entries.iter().any(|entry| {
                entry.tag != calcifer_macos_acl::TAG_DENY
                    || entry.flags != 0
                    || entry.permissions != calcifer_macos_acl::PERMISSION_DELETE
            })
        {
            return Err(corrupt());
        }
        Ok(())
    }

    fn decode_record(
        bytes: &[u8],
        workspace: &str,
        name: &str,
    ) -> Result<NativeBackgroundDetail, NativeBackgroundInspectionError> {
        validate_json_shape(bytes)?;
        let record: StoredBackgroundRecord =
            serde_json::from_slice(bytes).map_err(|_| corrupt())?;
        validate_record(record, workspace, name)
    }

    fn validate_json_shape(bytes: &[u8]) -> Result<(), NativeBackgroundInspectionError> {
        let context = JsonShapeContext {
            nodes: std::cell::Cell::new(0),
        };
        let mut deserializer = serde_json::Deserializer::from_slice(bytes);
        JsonShapeSeed {
            context: &context,
            container_depth: 0,
        }
        .deserialize(&mut deserializer)
        .map_err(|_| corrupt())?;
        deserializer.end().map_err(|_| corrupt())
    }

    struct JsonShapeContext {
        nodes: std::cell::Cell<usize>,
    }

    impl JsonShapeContext {
        fn consume_node<E: serde::de::Error>(&self) -> Result<(), E> {
            let Some(nodes) = self.nodes.get().checked_add(1) else {
                return Err(E::custom("background JSON node limit exceeded"));
            };
            if nodes > MAX_BACKGROUND_JSON_NODES {
                return Err(E::custom("background JSON node limit exceeded"));
            }
            self.nodes.set(nodes);
            Ok(())
        }

        fn enter_container<E: serde::de::Error>(depth: usize) -> Result<(), E> {
            if depth > MAX_BACKGROUND_JSON_DEPTH {
                return Err(E::custom("background JSON depth limit exceeded"));
            }
            Ok(())
        }
    }

    #[derive(Clone, Copy)]
    struct JsonShapeSeed<'a> {
        context: &'a JsonShapeContext,
        container_depth: usize,
    }

    impl<'de> DeserializeSeed<'de> for JsonShapeSeed<'_> {
        type Value = ();

        fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            self.context.consume_node()?;
            deserializer.deserialize_any(JsonShapeVisitor { seed: self })
        }
    }

    struct JsonShapeVisitor<'a> {
        seed: JsonShapeSeed<'a>,
    }

    impl<'de> Visitor<'de> for JsonShapeVisitor<'_> {
        type Value = ();

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a bounded JSON value")
        }

        fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
            Ok(())
        }

        fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
            Ok(())
        }

        fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
            Ok(())
        }

        fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
            Ok(())
        }

        fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
            Ok(())
        }

        fn visit_borrowed_str<E>(self, _: &'de str) -> Result<Self::Value, E> {
            Ok(())
        }

        fn visit_string<E>(self, _: String) -> Result<Self::Value, E> {
            Ok(())
        }

        fn visit_none<E>(self) -> Result<Self::Value, E> {
            Ok(())
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E> {
            Ok(())
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            self.seed.deserialize(deserializer)
        }

        fn visit_seq<A>(self, mut values: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let container_depth = self.seed.container_depth + 1;
            JsonShapeContext::enter_container(container_depth)?;
            let child = JsonShapeSeed {
                context: self.seed.context,
                container_depth,
            };
            while values.next_element_seed(child)?.is_some() {}
            Ok(())
        }

        fn visit_map<A>(self, mut values: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let container_depth = self.seed.container_depth + 1;
            JsonShapeContext::enter_container(container_depth)?;
            let child = JsonShapeSeed {
                context: self.seed.context,
                container_depth,
            };
            while values.next_key::<IgnoredAny>()?.is_some() {
                values.next_value_seed(child)?;
            }
            Ok(())
        }
    }

    fn validate_record(
        record: StoredBackgroundRecord,
        workspace: &str,
        name: &str,
    ) -> Result<NativeBackgroundDetail, NativeBackgroundInspectionError> {
        if record.version != 1
            || record.workspace != workspace
            || record.updated_at_ms < record.started_at_ms
            || record.command.is_empty()
            || invalid_string(&record.command, MAX_BACKGROUND_COMMAND_BYTES)
            || invalid_path(&record.workspace)
            || invalid_path(&record.cwd)
            || record.pid == Some(0)
            || record
                .server_url
                .as_deref()
                .is_some_and(|value| invalid_string(value, MAX_BACKGROUND_SERVER_URL_BYTES))
            || record
                .diagnostic
                .as_deref()
                .is_some_and(|value| invalid_string(value, MAX_BACKGROUND_DIAGNOSTIC_BYTES))
            || record_name(record.id) != name
            || !valid_exit_code(record.state, record.exit_code)
        {
            return Err(corrupt());
        }
        Ok(NativeBackgroundDetail {
            id: record.id,
            state: record.state,
            started_at_ms: record.started_at_ms,
            updated_at_ms: record.updated_at_ms,
            pid: record.pid,
            command: record.command,
            cwd: record.cwd,
            exit_code: record.exit_code,
            server_url: record.server_url,
            diagnostic: record.diagnostic,
        })
    }

    fn invalid_path(value: &str) -> bool {
        invalid_string(value, MAX_BACKGROUND_PATH_BYTES) || !is_canonical_absolute_path(value)
    }

    fn is_canonical_absolute_path(value: &str) -> bool {
        if !value.starts_with('/') {
            return false;
        }
        if value == "/" {
            return true;
        }
        !value.ends_with('/')
            && value
                .split('/')
                .skip(1)
                .all(|component| !component.is_empty() && component != "." && component != "..")
    }

    fn directory_open_flags() -> OFlags {
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NONBLOCK | noatime_flag()
    }

    fn record_open_flags() -> OFlags {
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK | noatime_flag()
    }

    #[cfg(target_os = "linux")]
    fn noatime_flag() -> OFlags {
        OFlags::from_bits_retain(libc::O_NOATIME as _)
    }

    #[cfg(target_os = "macos")]
    const fn noatime_flag() -> OFlags {
        OFlags::empty()
    }

    fn invalid_string(value: &str, maximum: usize) -> bool {
        value.len() > maximum || value.contains('\0')
    }

    fn valid_exit_code(state: NativeBackgroundState, code: Option<i32>) -> bool {
        match state {
            NativeBackgroundState::Running => code.is_none(),
            NativeBackgroundState::Exited => code == Some(0),
            NativeBackgroundState::Failed => code.is_some_and(|code| code != 0),
            NativeBackgroundState::Stopped
            | NativeBackgroundState::Dead
            | NativeBackgroundState::Stale => true,
        }
    }

    fn workspace_name(workspace: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(WORKSPACE_DIGEST_DOMAIN);
        hasher.update(workspace.as_bytes());
        format!("workspace-{:x}", hasher.finalize())
    }

    fn record_name(id: u64) -> String {
        let mut hasher = Sha256::new();
        hasher.update(RECORD_DIGEST_DOMAIN);
        hasher.update(id.to_be_bytes());
        format!("record-{:x}.json", hasher.finalize())
    }

    fn is_record_name(name: &[u8]) -> bool {
        name.len() == RECORD_PREFIX.len() + 64 + RECORD_SUFFIX.len()
            && name.starts_with(RECORD_PREFIX)
            && name.ends_with(RECORD_SUFFIX)
            && name[RECORD_PREFIX.len()..RECORD_PREFIX.len() + 64]
                .iter()
                .all(|byte| byte.is_ascii_digit() || matches!(*byte, b'a'..=b'f'))
    }

    fn is_rejected_type_error(error: rustix::io::Errno) -> bool {
        matches!(
            error,
            rustix::io::Errno::LOOP
                | rustix::io::Errno::ISDIR
                | rustix::io::Errno::NOTDIR
                | rustix::io::Errno::NXIO
                | rustix::io::Errno::OPNOTSUPP
        )
    }

    const fn corrupt() -> NativeBackgroundInspectionError {
        NativeBackgroundInspectionError::new(NativeBackgroundInspectionErrorKind::Corrupt)
    }

    #[cfg(test)]
    mod tests {
        use super::validate_json_shape;

        #[test]
        fn streaming_shape_validation_accepts_exact_depth_and_rejects_one_excess() {
            assert!(validate_json_shape(br"[[[[0]]]]").is_ok());
            assert!(validate_json_shape(br"[[[[[0]]]]]").is_err());
        }

        #[test]
        fn streaming_shape_validation_accepts_exact_nodes_and_rejects_one_excess() {
            let exact = format!("[{}]", vec!["0"; 63].join(","));
            let excess = format!("[{}]", vec!["0"; 64].join(","));
            assert!(validate_json_shape(exact.as_bytes()).is_ok());
            assert!(validate_json_shape(excess.as_bytes()).is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NativeBackgroundDetail, NativeBackgroundInspectionError,
        NativeBackgroundInspectionErrorKind, NativeBackgroundState,
    };

    #[test]
    fn state_names_are_stable() {
        assert_eq!(NativeBackgroundState::Running.as_str(), "running");
        assert_eq!(NativeBackgroundState::Exited.as_str(), "exited");
        assert_eq!(NativeBackgroundState::Failed.as_str(), "failed");
        assert_eq!(NativeBackgroundState::Stopped.as_str(), "stopped");
        assert_eq!(NativeBackgroundState::Dead.as_str(), "dead");
        assert_eq!(NativeBackgroundState::Stale.as_str(), "stale");
    }

    #[test]
    fn error_text_and_debug_are_fixed() {
        let cases = [
            (
                NativeBackgroundInspectionErrorKind::NotFound,
                "not_found",
                "native background record was not found",
            ),
            (
                NativeBackgroundInspectionErrorKind::Corrupt,
                "corrupt",
                "native background history is corrupt",
            ),
            (
                NativeBackgroundInspectionErrorKind::ResourceLimit,
                "resource_limit",
                "native background inspection reached a resource limit",
            ),
            (
                NativeBackgroundInspectionErrorKind::Unavailable,
                "unavailable",
                "native background persistence is unavailable",
            ),
            (
                NativeBackgroundInspectionErrorKind::UnsupportedPlatform,
                "unsupported_platform",
                "native background inspection is unsupported on this platform",
            ),
        ];
        for (kind, name, message) in cases {
            let error = NativeBackgroundInspectionError::new(kind);
            assert_eq!(kind.as_str(), name);
            assert_eq!(error.to_string(), message);
            assert_eq!(
                format!("{error:?}"),
                format!("NativeBackgroundInspectionError {{ kind: {kind:?} }}")
            );
        }
    }

    #[test]
    fn preview_truncates_at_utf8_boundary() {
        let command = format!("{}é", "a".repeat(255));
        let detail = NativeBackgroundDetail {
            id: 1,
            state: NativeBackgroundState::Running,
            started_at_ms: 1,
            updated_at_ms: 2,
            pid: None,
            command,
            cwd: "/workspace".to_owned(),
            exit_code: None,
            server_url: None,
            diagnostic: None,
        };
        let summary = detail.summary();
        assert_eq!(summary.command_preview().len(), 255);
        assert!(summary.preview_truncated());
    }
}
