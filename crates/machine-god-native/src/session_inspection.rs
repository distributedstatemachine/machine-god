use std::error::Error;
use std::fmt;

use machine_god_core::{BoxFuture, SessionId, SessionIncarnationId, SessionRevision};
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
use machine_god_core::{SessionRecord, SessionStore, SessionStoreError, SessionStoreErrorKind};
#[cfg(all(not(test), any(target_os = "linux", target_os = "macos")))]
use machine_god_core::{SessionStoreError, SessionStoreErrorKind};

use crate::NativeEnvironment;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::state_environment::{
    ProcessStateEnvironmentReader, StateEnvironmentReader, capture_state_environment,
};

/// Bounded structural projection of one validated durable session record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeSessionInspection {
    session_id: SessionId,
    incarnation_id: SessionIncarnationId,
    revision: SessionRevision,
    next_turn_sequence: u64,
    message_count: usize,
    metadata_entry_count: usize,
}

impl NativeSessionInspection {
    /// Returns the validated stored session identifier.
    #[must_use]
    pub const fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// Returns the validated durable incarnation identifier.
    #[must_use]
    pub const fn incarnation_id(&self) -> &SessionIncarnationId {
        &self.incarnation_id
    }

    /// Returns the positive durable optimistic-concurrency revision.
    #[must_use]
    pub const fn revision(&self) -> SessionRevision {
        self.revision
    }

    /// Returns the positive first never-reserved turn sequence.
    #[must_use]
    pub const fn next_turn_sequence(&self) -> u64 {
        self.next_turn_sequence
    }

    /// Returns the number of stored provider-neutral messages.
    #[must_use]
    pub const fn message_count(&self) -> usize {
        self.message_count
    }

    /// Returns the number of top-level metadata entries.
    #[must_use]
    pub const fn metadata_entry_count(&self) -> usize {
        self.metadata_entry_count
    }

    #[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
    fn from_record(record: SessionRecord) -> Self {
        let message_count = record.messages.len();
        let metadata_entry_count = record.metadata.len();
        Self {
            session_id: record.id,
            incarnation_id: record.incarnation_id,
            revision: record.revision,
            next_turn_sequence: record.next_turn_sequence,
            message_count,
            metadata_entry_count,
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn from_file_summary(summary: crate::session_store::FileSessionInspection) -> Self {
        Self {
            session_id: summary.session_id,
            incarnation_id: summary.incarnation_id,
            revision: summary.revision,
            next_turn_sequence: summary.next_turn_sequence,
            message_count: summary.message_count,
            metadata_entry_count: summary.metadata_entry_count,
        }
    }
}

/// Stable category for native session-inspection failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeSessionInspectionErrorKind {
    /// Native session inspection is not implemented on this target.
    UnsupportedPlatform,
    /// No usable state environment input was selected.
    InvalidEnvironment,
    /// The selected state-root hierarchy failed safety validation.
    UnsafeStateRoot,
    /// The selected hierarchy or exact record does not exist.
    NotFound,
    /// The exact canonical durable session record was corrupt.
    Corrupt,
    /// Native persistence was unavailable or ambiguous.
    Unavailable,
}

impl NativeSessionInspectionErrorKind {
    /// Returns the stable, machine-readable name of this category.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::InvalidEnvironment => "invalid_environment",
            Self::UnsafeStateRoot => "unsafe_state_root",
            Self::NotFound => "not_found",
            Self::Corrupt => "corrupt",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Fixed, redacted failure to inspect one native session.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct NativeSessionInspectionError {
    kind: NativeSessionInspectionErrorKind,
}

impl NativeSessionInspectionError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> NativeSessionInspectionErrorKind {
        self.kind
    }

    const fn new(kind: NativeSessionInspectionErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for NativeSessionInspectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeSessionInspectionError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for NativeSessionInspectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            NativeSessionInspectionErrorKind::UnsupportedPlatform => {
                "native session inspection is unsupported on this platform"
            }
            NativeSessionInspectionErrorKind::InvalidEnvironment => {
                "native session environment selection is invalid"
            }
            NativeSessionInspectionErrorKind::UnsafeStateRoot => {
                "native session state root is unsafe"
            }
            NativeSessionInspectionErrorKind::NotFound => "native session was not found",
            NativeSessionInspectionErrorKind::Corrupt => "native session record is corrupt",
            NativeSessionInspectionErrorKind::Unavailable => {
                "native session persistence is unavailable"
            }
        })
    }
}

impl Error for NativeSessionInspectionError {}

/// Inspects one session from an injected native environment snapshot.
///
/// Construction is effect-inert. On Linux and macOS, the first poll selects
/// the state root, opens only an already-existing fixed hierarchy, and loads
/// exactly the requested current-schema record. Missing hierarchy components
/// and a missing exact record are not created. Filesystem work may block the
/// polling thread.
#[must_use]
pub fn inspect_native_session(
    environment: NativeEnvironment,
    id: SessionId,
) -> BoxFuture<'static, Result<NativeSessionInspection, NativeSessionInspectionError>> {
    Box::pin(async move { inspect_native_session_polled(&environment, id) })
}

/// Captures state-only process environment inputs and inspects one session.
///
/// Construction is effect-inert. Environment capture and any supported native
/// filesystem work occur only after the returned future is first polled.
#[must_use]
pub fn inspect_process_session(
    id: SessionId,
) -> BoxFuture<'static, Result<NativeSessionInspection, NativeSessionInspectionError>> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        inspect_process_session_with_reader(ProcessStateEnvironmentReader, id)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    Box::pin(async move {
        let environment = NativeEnvironment::new(None, None, None);
        inspect_native_session_polled(&environment, id)
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn inspect_process_session_with_reader<R>(
    mut reader: R,
    id: SessionId,
) -> BoxFuture<'static, Result<NativeSessionInspection, NativeSessionInspectionError>>
where
    R: StateEnvironmentReader + Send + 'static,
{
    Box::pin(async move {
        let environment = capture_state_environment(&mut reader);
        inspect_native_session_polled(&environment, id)
    })
}

fn inspect_native_session_polled(
    environment: &NativeEnvironment,
    id: SessionId,
) -> Result<NativeSessionInspection, NativeSessionInspectionError> {
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (environment, id);
        Err(NativeSessionInspectionError::new(
            NativeSessionInspectionErrorKind::UnsupportedPlatform,
        ))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let store = crate::root_selection::open_existing_session_store(environment)
            .map_err(map_root_error)?
            .ok_or_else(not_found)?;
        let summary = store
            .inspect_session_summary(id)
            .map_err(map_store_error)?
            .ok_or_else(not_found)?;
        Ok(NativeSessionInspection::from_file_summary(summary))
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
fn inspect_session_from_store<S>(
    store: &S,
    id: SessionId,
) -> Result<NativeSessionInspection, NativeSessionInspectionError>
where
    S: SessionStore + ?Sized,
{
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    let mut load = store.load(id);
    let mut context = Context::from_waker(Waker::noop());
    let record = match Future::poll(load.as_mut(), &mut context) {
        Poll::Ready(result) => result.map_err(map_store_error)?,
        Poll::Pending => {
            return Err(NativeSessionInspectionError::new(
                NativeSessionInspectionErrorKind::Unavailable,
            ));
        }
    }
    .ok_or_else(not_found)?;
    Ok(NativeSessionInspection::from_record(record))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_root_error(
    error: crate::root_selection::ExistingSessionStoreError,
) -> NativeSessionInspectionError {
    let kind = match error {
        crate::root_selection::ExistingSessionStoreError::InvalidEnvironment => {
            NativeSessionInspectionErrorKind::InvalidEnvironment
        }
        crate::root_selection::ExistingSessionStoreError::UnsafeStateRoot => {
            NativeSessionInspectionErrorKind::UnsafeStateRoot
        }
        crate::root_selection::ExistingSessionStoreError::Unavailable => {
            NativeSessionInspectionErrorKind::Unavailable
        }
    };
    NativeSessionInspectionError::new(kind)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_store_error(error: SessionStoreError) -> NativeSessionInspectionError {
    let kind = match error.kind {
        SessionStoreErrorKind::NotFound => NativeSessionInspectionErrorKind::NotFound,
        SessionStoreErrorKind::Corrupt => NativeSessionInspectionErrorKind::Corrupt,
        SessionStoreErrorKind::Conflict
        | SessionStoreErrorKind::Unavailable
        | SessionStoreErrorKind::Other
        | _ => NativeSessionInspectionErrorKind::Unavailable,
    };
    drop(error);
    NativeSessionInspectionError::new(kind)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
const fn not_found() -> NativeSessionInspectionError {
    NativeSessionInspectionError::new(NativeSessionInspectionErrorKind::NotFound)
}

#[cfg(test)]
mod tests {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::ffi::OsString;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::future::Future;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::os::unix::fs::{PermissionsExt, symlink};
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::path::{Path, PathBuf};
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::pin::Pin;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::sync::atomic::{AtomicU64, Ordering};
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::sync::{Arc, Mutex};
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::task::{Context, Poll, Wake, Waker};
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::{fs, io};

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use machine_god_core::{
        BoxFuture, ContentBlock, Message, Role, SessionRecord, SessionStore, SessionStoreError,
        SessionStoreErrorKind, ToolCall, ToolCallId, ToolName, ToolOutput,
    };
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use serde_json::json;

    use super::*;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use crate::FileSessionStore;

    #[test]
    fn error_kinds_have_stable_names_and_redacted_messages() {
        let cases = [
            (
                NativeSessionInspectionErrorKind::UnsupportedPlatform,
                "unsupported_platform",
                "native session inspection is unsupported on this platform",
            ),
            (
                NativeSessionInspectionErrorKind::InvalidEnvironment,
                "invalid_environment",
                "native session environment selection is invalid",
            ),
            (
                NativeSessionInspectionErrorKind::UnsafeStateRoot,
                "unsafe_state_root",
                "native session state root is unsafe",
            ),
            (
                NativeSessionInspectionErrorKind::NotFound,
                "not_found",
                "native session was not found",
            ),
            (
                NativeSessionInspectionErrorKind::Corrupt,
                "corrupt",
                "native session record is corrupt",
            ),
            (
                NativeSessionInspectionErrorKind::Unavailable,
                "unavailable",
                "native session persistence is unavailable",
            ),
        ];
        for (kind, name, message) in cases {
            let error = NativeSessionInspectionError::new(kind);
            assert_eq!(kind.as_str(), name);
            assert_eq!(error.kind(), kind);
            assert_eq!(error.to_string(), message);
            let rendered = format!("{error:?} {error}");
            assert!(!rendered.contains('/'));
            assert!(!rendered.contains("PRIVATE"));
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn supported_targets_reach_state_selection() {
        let error = futures_executor::block_on(inspect_native_session(
            NativeEnvironment::new(None, None, None),
            SessionId::new("alpha").unwrap(),
        ))
        .unwrap_err();
        assert_eq!(
            error.kind(),
            NativeSessionInspectionErrorKind::InvalidEnvironment
        );
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    #[test]
    fn unsupported_targets_fail_without_state_selection() {
        let error = futures_executor::block_on(inspect_native_session(
            NativeEnvironment::new(None, None, None),
            SessionId::new("alpha").unwrap(),
        ))
        .unwrap_err();
        assert_eq!(
            error.kind(),
            NativeSessionInspectionErrorKind::UnsupportedPlatform
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    struct RecordingEnvironmentReader {
        xdg_state_home: Option<OsString>,
        home: Option<OsString>,
        requests: Arc<Mutex<Vec<&'static str>>>,
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    impl RecordingEnvironmentReader {
        fn new(xdg_state_home: Option<OsString>, home: Option<OsString>) -> Self {
            Self {
                xdg_state_home,
                home,
                requests: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn request_log(&self) -> Arc<Mutex<Vec<&'static str>>> {
            Arc::clone(&self.requests)
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    impl StateEnvironmentReader for RecordingEnvironmentReader {
        fn read(&mut self, name: &'static str) -> Option<OsString> {
            self.requests.lock().unwrap().push(name);
            match name {
                "XDG_STATE_HOME" => self.xdg_state_home.take(),
                "HOME" => self.home.take(),
                unexpected => panic!("unexpected environment request: {unexpected}"),
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    struct TempDirectory(PathBuf);

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    impl TempDirectory {
        fn new(label: &str) -> Self {
            loop {
                let sequence = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "machine-god-session-inspection-{label}-{}-{sequence}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => {
                        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
                        return Self(path);
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("create test directory: {error}"),
                }
            }
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    struct NoopWake;

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn ready<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(NoopWake));
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        match Future::poll(Pin::as_mut(&mut future), &mut context) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("native test future unexpectedly remained pending"),
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn id(value: &str) -> SessionId {
        SessionId::new(value).unwrap()
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn incarnation(value: &str) -> SessionIncarnationId {
        SessionIncarnationId::new(value).unwrap()
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn create_state_root(base: &Path, mode: u32) -> PathBuf {
        let root = base.join(crate::STATE_NAMESPACE);
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(mode)).unwrap();
        root
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn environment(base: &Path) -> NativeEnvironment {
        NativeEnvironment::new(None, Some(base.as_os_str().to_owned()), None)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn entry_with_suffix(root: &Path, suffix: &str) -> PathBuf {
        let matches = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(suffix))
            })
            .collect::<Vec<_>>();
        assert_eq!(matches.len(), 1, "expected one {suffix} entry");
        matches.into_iter().next().unwrap()
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn sibling_with_suffix(path: &Path, old_suffix: &str, new_suffix: &str) -> PathBuf {
        let name = path.file_name().unwrap().to_str().unwrap();
        path.with_file_name(format!(
            "{}{}",
            name.strip_suffix(old_suffix).unwrap(),
            new_suffix
        ))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn save_record(store: &FileSessionStore, record: SessionRecord) -> SessionRevision {
        ready(store.save(record, None)).unwrap()
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn process_capture_is_first_poll_state_only_and_lazy() {
        for (xdg, home, expected_requests) in [
            (
                Some(OsString::from("relative-state")),
                Some(OsString::from("/must-not-be-read")),
                vec!["XDG_STATE_HOME"],
            ),
            (Some(OsString::new()), None, vec!["XDG_STATE_HOME", "HOME"]),
            (None, None, vec!["XDG_STATE_HOME", "HOME"]),
        ] {
            let reader = RecordingEnvironmentReader::new(xdg, home);
            let requests = reader.request_log();
            let future = inspect_process_session_with_reader(reader, id("alpha"));
            assert!(requests.lock().unwrap().is_empty());

            let error = ready(future).unwrap_err();
            assert_eq!(
                error.kind(),
                NativeSessionInspectionErrorKind::InvalidEnvironment
            );
            assert_eq!(*requests.lock().unwrap(), expected_requests);
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn non_unicode_xdg_is_selected_without_home_fallback() {
        use std::os::unix::ffi::OsStringExt;

        let reader = RecordingEnvironmentReader::new(
            Some(OsString::from_vec(b"/state-\xff".to_vec())),
            Some(OsString::from("/must-not-be-read")),
        );
        let requests = reader.request_log();
        let error = ready(inspect_process_session_with_reader(reader, id("alpha"))).unwrap_err();

        assert_eq!(
            error.kind(),
            NativeSessionInspectionErrorKind::InvalidEnvironment
        );
        assert_eq!(*requests.lock().unwrap(), ["XDG_STATE_HOME"]);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn process_capture_home_fallback_reaches_the_exact_existing_store() {
        let home = TempDirectory::new("home-fallback");
        let local = home.path().join(".local");
        let state = local.join("state");
        let root = state.join(crate::STATE_NAMESPACE);
        for directory in [&local, &state, &root] {
            fs::create_dir(directory).unwrap();
        }
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let store = FileSessionStore::open(&root).unwrap();
        save_record(
            &store,
            SessionRecord::empty(id("alpha"), incarnation("home-life")),
        );
        let reader = RecordingEnvironmentReader::new(
            Some(OsString::new()),
            Some(home.path().as_os_str().to_owned()),
        );
        let requests = reader.request_log();

        let inspection = ready(inspect_process_session_with_reader(reader, id("alpha"))).unwrap();

        assert_eq!(inspection.incarnation_id().as_str(), "home-life");
        assert_eq!(*requests.lock().unwrap(), ["XDG_STATE_HOME", "HOME"]);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn injected_inspection_is_inert_until_first_poll() {
        let temporary = TempDirectory::new("inert");
        let root = create_state_root(temporary.path(), 0o700);
        let store = FileSessionStore::open(&root).unwrap();
        save_record(
            &store,
            SessionRecord::empty(id("alpha"), incarnation("inc-alpha")),
        );
        let data = entry_with_suffix(&root, ".json");
        let lock = sibling_with_suffix(&data, ".json", ".lock");
        fs::remove_file(&lock).unwrap();
        let persisted_bytes = fs::read(&data).unwrap();

        let future = inspect_native_session(environment(temporary.path()), id("alpha"));
        assert!(!lock.exists());
        let inspection = ready(future).unwrap();

        assert_eq!(inspection.session_id().as_str(), "alpha");
        assert!(lock.exists());
        assert_eq!(fs::read(data).unwrap(), persisted_bytes);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn exact_projection_retains_only_structural_values_and_private_lock() {
        let temporary = TempDirectory::new("projection");
        let root = create_state_root(temporary.path(), 0o700);
        let store = FileSessionStore::open(&root).unwrap();
        let mut record = SessionRecord::empty(id("alpha"), incarnation("incarnation-alpha"));
        record.next_turn_sequence = 4;
        record.messages = vec![
            Message::text(Role::User, "PRIVATE_TRANSCRIPT_ONE"),
            Message::text(Role::Assistant, "PRIVATE_TRANSCRIPT_TWO"),
            Message::text(Role::User, "PRIVATE_TRANSCRIPT_THREE"),
        ];
        record
            .metadata
            .insert("PRIVATE_METADATA_KEY".to_owned(), json!("PRIVATE_VALUE"));
        record.metadata.insert("second".to_owned(), json!(2));
        let mut revision = save_record(&store, record);
        while revision.0 < 7 {
            let current = ready(store.load(id("alpha"))).unwrap().unwrap();
            revision = ready(store.save(current, Some(revision))).unwrap();
        }
        let data = entry_with_suffix(&root, ".json");
        let lock = sibling_with_suffix(&data, ".json", ".lock");
        fs::remove_file(&lock).unwrap();
        let persisted_bytes = fs::read(&data).unwrap();

        let inspection = ready(inspect_native_session(
            environment(temporary.path()),
            id("alpha"),
        ))
        .unwrap();

        assert_eq!(inspection.session_id().as_str(), "alpha");
        assert_eq!(inspection.incarnation_id().as_str(), "incarnation-alpha");
        assert_eq!(inspection.revision(), SessionRevision(7));
        assert_eq!(inspection.next_turn_sequence(), 4);
        assert_eq!(inspection.message_count(), 3);
        assert_eq!(inspection.metadata_entry_count(), 2);
        let rendered = format!("{inspection:?}");
        assert!(!rendered.contains("PRIVATE_TRANSCRIPT"));
        assert!(!rendered.contains("PRIVATE_METADATA"));
        assert!(!rendered.contains("PRIVATE_VALUE"));
        assert_eq!(
            fs::metadata(lock).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read(data).unwrap(), persisted_bytes);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn near_file_cap_transcript_streams_through_fixed_read_requests() {
        let temporary = TempDirectory::new("near-cap-stream");
        let store = FileSessionStore::open(temporary.path()).unwrap();
        let payload = "x".repeat(crate::MAX_FILE_SESSION_BYTES - 512);
        let mut record = SessionRecord::empty(id("alpha"), incarnation("inc-alpha"));
        record.messages.push(Message::text(Role::User, payload));
        save_record(&store, record);
        let data = entry_with_suffix(temporary.path(), ".json");
        let file_bytes = usize::try_from(fs::metadata(&data).unwrap().len()).unwrap();
        assert!(file_bytes > crate::MAX_FILE_SESSION_BYTES - 1_024);

        let summary = store.inspect_session_summary(id("alpha")).unwrap().unwrap();

        assert_eq!(summary.message_count, 1);
        assert_eq!(summary.bytes_read, file_bytes);
        assert!(summary.max_read_request <= 4 * 1_024);
        assert_eq!(fs::metadata(data).unwrap().len(), file_bytes as u64);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn inspection_ignores_engine_message_and_metadata_size_configuration() {
        let temporary = TempDirectory::new("historical-large-record");
        let store = FileSessionStore::open(temporary.path()).unwrap();
        let mut record = SessionRecord::empty(id("alpha"), incarnation("inc-alpha"));
        record.messages = (0..5_000)
            .map(|_| Message {
                role: Role::User,
                content: Vec::new(),
            })
            .collect();
        record
            .metadata
            .insert("large".to_owned(), json!("m".repeat(300 * 1_024)));
        record.metadata.insert("second".to_owned(), json!(true));
        save_record(&store, record);

        let summary = store.inspect_session_summary(id("alpha")).unwrap().unwrap();

        assert_eq!(summary.message_count, 5_000);
        assert_eq!(summary.metadata_entry_count, 2);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn every_current_content_block_shape_is_stream_validated() {
        let temporary = TempDirectory::new("content-schema");
        let store = FileSessionStore::open(temporary.path()).unwrap();
        let mut record = SessionRecord::empty(id("alpha"), incarnation("inc-alpha"));
        record.messages.push(Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::Text {
                    text: "private-🌍".to_owned(),
                },
                ContentBlock::Json {
                    value: json!({"array": [null, true, 17, "value"]}),
                },
                ContentBlock::ToolCall {
                    call: ToolCall {
                        id: ToolCallId::new("call-1").unwrap(),
                        name: ToolName::new("read_file").unwrap(),
                        arguments: json!({"path": "private.txt"}),
                    },
                },
                ContentBlock::ToolResult {
                    call_id: ToolCallId::new("call-1").unwrap(),
                    output: ToolOutput {
                        content: json!({"ok": true}),
                        is_error: false,
                    },
                },
            ],
        });
        save_record(&store, record);

        let summary = store.inspect_session_summary(id("alpha")).unwrap().unwrap();

        assert_eq!(summary.message_count, 1);
        assert_eq!(summary.metadata_entry_count, 0);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn aggregate_json_node_overflow_is_rejected_during_streaming() {
        let temporary = TempDirectory::new("json-node-overflow");
        let store = FileSessionStore::open(temporary.path()).unwrap();
        save_record(
            &store,
            SessionRecord::empty(id("alpha"), incarnation("inc-alpha")),
        );
        let data = entry_with_suffix(temporary.path(), ".json");
        let persisted = String::from_utf8(fs::read(&data).unwrap()).unwrap();
        let nodes = std::iter::repeat_n("null", 65_536)
            .collect::<Vec<_>>()
            .join(",");
        let corrupt = persisted.replacen(
            "\"metadata\":{}",
            &format!("\"metadata\":{{\"overflow\":[{nodes}]}}"),
            1,
        );
        assert_ne!(corrupt, persisted);
        fs::write(&data, corrupt).unwrap();

        let error = store.inspect_session_summary(id("alpha")).unwrap_err();

        assert_eq!(error.kind, SessionStoreErrorKind::Corrupt);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn envelope_types_unknowns_duplicates_and_metadata_duplicates_are_corrupt() {
        let temporary = TempDirectory::new("strict-summary-schema");
        let store = FileSessionStore::open(temporary.path()).unwrap();
        save_record(
            &store,
            SessionRecord::empty(id("alpha"), incarnation("inc-alpha")),
        );
        let data = entry_with_suffix(temporary.path(), ".json");
        let persisted = String::from_utf8(fs::read(&data).unwrap()).unwrap();
        let record_json =
            serde_json::to_string(&ready(store.load(id("alpha"))).unwrap().unwrap()).unwrap();
        let cases = [
            persisted.replacen("\"schema_version\":1", "\"schema_version\":\"1\"", 1),
            format!(
                "{},\"unknown\":true}}",
                persisted.strip_suffix('}').unwrap()
            ),
            format!("{{\"schema_version\":1,\"schema_version\":1,\"record\":{record_json}}}"),
            persisted.replacen("\"metadata\":{}", "\"metadata\":{\"same\":1,\"same\":2}", 1),
        ];

        for contents in cases {
            fs::write(&data, contents).unwrap();
            let error = store.inspect_session_summary(id("alpha")).unwrap_err();
            assert_eq!(error.kind, SessionStoreErrorKind::Corrupt);
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn missing_hierarchy_and_record_are_not_found_and_create_nothing() {
        let temporary = TempDirectory::new("missing");
        let missing_base = temporary.path().join("absent");
        let error = ready(inspect_native_session(
            environment(&missing_base),
            id("alpha"),
        ))
        .unwrap_err();
        assert_eq!(error.kind(), NativeSessionInspectionErrorKind::NotFound);
        assert!(!missing_base.exists());

        let suffix_base = TempDirectory::new("missing-suffix");
        let error = ready(inspect_native_session(
            environment(suffix_base.path()),
            id("alpha"),
        ))
        .unwrap_err();
        assert_eq!(error.kind(), NativeSessionInspectionErrorKind::NotFound);
        assert!(!suffix_base.path().join(crate::STATE_NAMESPACE).exists());

        let record_base = TempDirectory::new("missing-record");
        let root = create_state_root(record_base.path(), 0o700);
        let error = ready(inspect_native_session(
            environment(record_base.path()),
            id("alpha"),
        ))
        .unwrap_err();
        assert_eq!(error.kind(), NativeSessionInspectionErrorKind::NotFound);
        assert!(fs::read_dir(root).unwrap().next().is_none());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn invalid_and_unsafe_roots_keep_distinct_categories() {
        let invalid = ready(inspect_native_session(
            NativeEnvironment::new(
                None,
                Some(OsString::from("relative")),
                Some(OsString::from("/must-not-fallback")),
            ),
            id("alpha"),
        ))
        .unwrap_err();
        assert_eq!(
            invalid.kind(),
            NativeSessionInspectionErrorKind::InvalidEnvironment
        );

        let permissive = TempDirectory::new("unsafe-mode");
        create_state_root(permissive.path(), 0o755);
        let error = ready(inspect_native_session(
            environment(permissive.path()),
            id("alpha"),
        ))
        .unwrap_err();
        assert_eq!(
            error.kind(),
            NativeSessionInspectionErrorKind::UnsafeStateRoot
        );

        for kind in ["file", "symlink"] {
            let base = TempDirectory::new(kind);
            let selected = base.path().join(crate::STATE_NAMESPACE);
            if kind == "file" {
                fs::write(&selected, b"PRIVATE_ROOT_BYTES").unwrap();
            } else {
                let target = base.path().join("PRIVATE_ROOT_TARGET");
                fs::create_dir(&target).unwrap();
                symlink(&target, &selected).unwrap();
            }
            let error = ready(inspect_native_session(
                environment(base.path()),
                id("alpha"),
            ))
            .unwrap_err();
            assert_eq!(
                error.kind(),
                NativeSessionInspectionErrorKind::UnsafeStateRoot
            );
            assert!(!format!("{error:?} {error}").contains("PRIVATE"));
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn corrupt_oversized_wrong_id_and_nonregular_records_map_to_corrupt() {
        for kind in ["bytes", "oversized", "wrong-id", "directory", "symlink"] {
            let temporary = TempDirectory::new(kind);
            let root = create_state_root(temporary.path(), 0o700);
            let store = FileSessionStore::open(&root).unwrap();
            save_record(
                &store,
                SessionRecord::empty(id("alpha"), incarnation("inc-alpha")),
            );
            let data = entry_with_suffix(&root, ".json");
            match kind {
                "bytes" => fs::write(&data, b"PRIVATE_MALFORMED_RECORD").unwrap(),
                "oversized" => {
                    fs::write(&data, vec![b'x'; crate::MAX_FILE_SESSION_BYTES + 1]).unwrap();
                }
                "wrong-id" => {
                    let source = TempDirectory::new("wrong-id-source");
                    let source_store = FileSessionStore::open(source.path()).unwrap();
                    save_record(
                        &source_store,
                        SessionRecord::empty(id("beta"), incarnation("inc-beta")),
                    );
                    fs::copy(entry_with_suffix(source.path(), ".json"), &data).unwrap();
                }
                "directory" => {
                    fs::remove_file(&data).unwrap();
                    fs::create_dir(&data).unwrap();
                }
                "symlink" => {
                    fs::remove_file(&data).unwrap();
                    let target = root.join("PRIVATE_TARGET");
                    fs::write(&target, b"PRIVATE_TARGET_BYTES").unwrap();
                    symlink(&target, &data).unwrap();
                }
                _ => unreachable!(),
            }

            let error = ready(inspect_native_session(
                environment(temporary.path()),
                id("alpha"),
            ))
            .unwrap_err();
            assert_eq!(error.kind(), NativeSessionInspectionErrorKind::Corrupt);
            assert!(!format!("{error:?} {error}").contains("PRIVATE"));
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn retained_store_descriptor_prevents_root_replacement_redirection() {
        let temporary = TempDirectory::new("retained-root");
        let root = temporary.path().join("root");
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let store = FileSessionStore::open(&root).unwrap();
        let mut original = SessionRecord::empty(id("alpha"), incarnation("original-life"));
        original.metadata.insert("one".to_owned(), json!(1));
        save_record(&store, original);

        let moved = temporary.path().join("moved-root");
        fs::rename(&root, &moved).unwrap();
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let replacement_store = FileSessionStore::open(&root).unwrap();
        let replacement = SessionRecord::empty(id("alpha"), incarnation("replacement-life"));
        save_record(&replacement_store, replacement);

        let summary = store.inspect_session_summary(id("alpha")).unwrap().unwrap();
        assert_eq!(summary.incarnation_id.as_str(), "original-life");
        assert_eq!(summary.metadata_entry_count, 1);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    struct PendingStore;

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    impl SessionStore for PendingStore {
        fn load(
            &self,
            _id: SessionId,
        ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
            Box::pin(std::future::pending())
        }

        fn save(
            &self,
            _record: SessionRecord,
            _expected_revision: Option<SessionRevision>,
        ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
            Box::pin(std::future::pending())
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn pending_load_and_store_categories_are_closed_and_redacted() {
        let pending = inspect_session_from_store(&PendingStore, id("alpha")).unwrap_err();
        assert_eq!(
            pending.kind(),
            NativeSessionInspectionErrorKind::Unavailable
        );

        for (store_kind, expected) in [
            (
                SessionStoreErrorKind::NotFound,
                NativeSessionInspectionErrorKind::NotFound,
            ),
            (
                SessionStoreErrorKind::Corrupt,
                NativeSessionInspectionErrorKind::Corrupt,
            ),
            (
                SessionStoreErrorKind::Conflict,
                NativeSessionInspectionErrorKind::Unavailable,
            ),
            (
                SessionStoreErrorKind::Unavailable,
                NativeSessionInspectionErrorKind::Unavailable,
            ),
            (
                SessionStoreErrorKind::Other,
                NativeSessionInspectionErrorKind::Unavailable,
            ),
        ] {
            let mapped = map_store_error(SessionStoreError::new(
                store_kind,
                "PRIVATE_CODE",
                "PRIVATE_MESSAGE",
                true,
            ));
            assert_eq!(mapped.kind(), expected);
            assert!(!format!("{mapped:?} {mapped}").contains("PRIVATE"));
        }

        for (root_error, expected) in [
            (
                crate::root_selection::ExistingSessionStoreError::InvalidEnvironment,
                NativeSessionInspectionErrorKind::InvalidEnvironment,
            ),
            (
                crate::root_selection::ExistingSessionStoreError::UnsafeStateRoot,
                NativeSessionInspectionErrorKind::UnsafeStateRoot,
            ),
            (
                crate::root_selection::ExistingSessionStoreError::Unavailable,
                NativeSessionInspectionErrorKind::Unavailable,
            ),
        ] {
            assert_eq!(map_root_error(root_error).kind(), expected);
        }
    }
}
