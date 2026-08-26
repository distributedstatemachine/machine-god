use std::error::Error;
use std::fmt;

use machine_god_core::{BoxFuture, SessionId};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use machine_god_core::{SessionStoreError, SessionStoreErrorKind};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::FileSessionStore;
use crate::NativeEnvironment;

/// Bounded durable session-ID observation from one native store scan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeSessionList {
    session_ids: Vec<SessionId>,
    truncated: bool,
}

impl NativeSessionList {
    /// Returns the observed session IDs in ascending identifier order.
    #[must_use]
    pub fn session_ids(&self) -> &[SessionId] {
        &self.session_ids
    }

    /// Consumes this result and returns its observed session IDs.
    #[must_use]
    pub fn into_session_ids(self) -> Vec<SessionId> {
        self.session_ids
    }

    /// Reports whether a work or result bound prevented an exhaustive scan.
    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn empty() -> Self {
        Self {
            session_ids: Vec::new(),
            truncated: false,
        }
    }
}

/// Stable category for native session-listing failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeSessionListingErrorKind {
    /// Native session listing is not implemented on this target.
    UnsupportedPlatform,
    /// No usable state environment input was selected.
    InvalidEnvironment,
    /// The selected state-root hierarchy failed safety validation.
    UnsafeStateRoot,
    /// A canonical durable session record was corrupt.
    Corrupt,
    /// Native persistence was unavailable or ambiguous.
    Unavailable,
}

impl NativeSessionListingErrorKind {
    /// Returns the stable, machine-readable name of this category.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::InvalidEnvironment => "invalid_environment",
            Self::UnsafeStateRoot => "unsafe_state_root",
            Self::Corrupt => "corrupt",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Fixed, redacted failure to list native sessions.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct NativeSessionListingError {
    kind: NativeSessionListingErrorKind,
}

impl NativeSessionListingError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> NativeSessionListingErrorKind {
        self.kind
    }

    const fn new(kind: NativeSessionListingErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for NativeSessionListingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeSessionListingError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for NativeSessionListingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            NativeSessionListingErrorKind::UnsupportedPlatform => {
                "native session listing is unsupported on this platform"
            }
            NativeSessionListingErrorKind::InvalidEnvironment => {
                "native session environment selection is invalid"
            }
            NativeSessionListingErrorKind::UnsafeStateRoot => "native session state root is unsafe",
            NativeSessionListingErrorKind::Corrupt => "native session record is corrupt",
            NativeSessionListingErrorKind::Unavailable => {
                "native session persistence is unavailable"
            }
        })
    }
}

impl Error for NativeSessionListingError {}

/// Lists sessions from an injected native environment snapshot.
///
/// Construction is effect-inert. On Linux and macOS, the first poll selects
/// the state root, opens only an already-existing fixed hierarchy, and runs the
/// same bounded store scan used by `NativeSessionLifecycle`. Missing
/// state directories produce an empty, nontruncated result and are never
/// created. Filesystem work may block the polling thread.
#[must_use]
pub fn list_native_sessions(
    environment: NativeEnvironment,
) -> BoxFuture<'static, Result<NativeSessionList, NativeSessionListingError>> {
    Box::pin(async move { list_native_sessions_polled(&environment) })
}

/// Captures the process environment and lists sessions on first poll.
///
/// Construction is effect-inert. Environment capture and any supported native
/// filesystem work occur only after the returned future is first polled.
#[must_use]
pub fn list_process_sessions()
-> BoxFuture<'static, Result<NativeSessionList, NativeSessionListingError>> {
    Box::pin(async move {
        let environment = NativeEnvironment::from_process();
        list_native_sessions_polled(&environment)
    })
}

fn list_native_sessions_polled(
    environment: &NativeEnvironment,
) -> Result<NativeSessionList, NativeSessionListingError> {
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = environment;
        Err(NativeSessionListingError::new(
            NativeSessionListingErrorKind::UnsupportedPlatform,
        ))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let store = crate::root_selection::open_existing_session_store(environment)
            .map_err(map_root_error)?;
        store.as_ref().map_or_else(
            || Ok(NativeSessionList::empty()),
            |store| list_sessions_from_store(store).map_err(map_store_error),
        )
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn list_sessions_from_store(
    store: &FileSessionStore,
) -> Result<NativeSessionList, SessionStoreError> {
    let listing = store.list_session_ids()?;
    Ok(NativeSessionList {
        session_ids: listing.session_ids,
        truncated: listing.truncated,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_root_error(
    error: crate::root_selection::ExistingSessionStoreError,
) -> NativeSessionListingError {
    let kind = match error {
        crate::root_selection::ExistingSessionStoreError::InvalidEnvironment => {
            NativeSessionListingErrorKind::InvalidEnvironment
        }
        crate::root_selection::ExistingSessionStoreError::UnsafeStateRoot => {
            NativeSessionListingErrorKind::UnsafeStateRoot
        }
        crate::root_selection::ExistingSessionStoreError::Unavailable => {
            NativeSessionListingErrorKind::Unavailable
        }
    };
    NativeSessionListingError::new(kind)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_store_error(error: SessionStoreError) -> NativeSessionListingError {
    let kind = match error.kind {
        SessionStoreErrorKind::Corrupt => NativeSessionListingErrorKind::Corrupt,
        SessionStoreErrorKind::NotFound
        | SessionStoreErrorKind::Conflict
        | SessionStoreErrorKind::Unavailable
        | SessionStoreErrorKind::Other
        | _ => NativeSessionListingErrorKind::Unavailable,
    };
    drop(error);
    NativeSessionListingError::new(kind)
}

#[cfg(test)]
mod tests {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::ffi::OsString;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::path::{Path, PathBuf};
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::sync::atomic::{AtomicU64, Ordering};
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::{fs, os::unix::fs::PermissionsExt};

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use futures_executor::block_on;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use machine_god_core::{SessionIncarnationId, SessionRecord, SessionStore};

    use super::*;

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    struct TempDirectory(PathBuf);

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    impl TempDirectory {
        fn new() -> Self {
            loop {
                let sequence = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "machine-god-process-session-listing-{}-{sequence}",
                    std::process::id()
                ));
                match std::fs::create_dir(&path) {
                    Ok(()) => {
                        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
                        return Self(path);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
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
    fn create_xdg_state_root(base: &Path, mode: u32) -> PathBuf {
        let root = base.join(crate::STATE_NAMESPACE);
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(mode)).unwrap();
        root
    }

    #[test]
    fn error_kinds_have_stable_names_and_redacted_messages() {
        let cases = [
            (
                NativeSessionListingErrorKind::UnsupportedPlatform,
                "unsupported_platform",
                "native session listing is unsupported on this platform",
            ),
            (
                NativeSessionListingErrorKind::InvalidEnvironment,
                "invalid_environment",
                "native session environment selection is invalid",
            ),
            (
                NativeSessionListingErrorKind::UnsafeStateRoot,
                "unsafe_state_root",
                "native session state root is unsafe",
            ),
            (
                NativeSessionListingErrorKind::Corrupt,
                "corrupt",
                "native session record is corrupt",
            ),
            (
                NativeSessionListingErrorKind::Unavailable,
                "unavailable",
                "native session persistence is unavailable",
            ),
        ];
        for (kind, name, message) in cases {
            let error = NativeSessionListingError::new(kind);
            assert_eq!(kind.as_str(), name);
            assert_eq!(error.kind(), kind);
            assert_eq!(error.to_string(), message);
            assert!(!format!("{error:?}").contains('/'));
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn absent_environment_is_invalid() {
        let error = block_on(list_native_sessions(NativeEnvironment::new(
            None, None, None,
        )))
        .unwrap_err();
        assert_eq!(
            error.kind(),
            NativeSessionListingErrorKind::InvalidEnvironment
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn missing_selected_state_base_is_empty_and_not_created() {
        let temporary = TempDirectory::new();
        let state_base = temporary.path().join("missing");
        let future = list_native_sessions(NativeEnvironment::new(
            None,
            Some(state_base.clone().into_os_string()),
            None,
        ));
        assert!(!state_base.exists());

        let listing = block_on(future).unwrap();
        assert!(listing.session_ids().is_empty());
        assert!(!listing.truncated());
        assert!(!state_base.exists());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn invalid_selected_xdg_value_does_not_fall_back_to_home() {
        let temporary = TempDirectory::new();
        let environment = NativeEnvironment::new(
            None,
            Some(OsString::from("relative")),
            Some(temporary.path().as_os_str().to_owned()),
        );
        let error = block_on(list_native_sessions(environment)).unwrap_err();
        assert_eq!(
            error.kind(),
            NativeSessionListingErrorKind::InvalidEnvironment
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn missing_fixed_suffix_is_empty_and_not_created() {
        let state_base = TempDirectory::new();
        let listing = block_on(list_native_sessions(NativeEnvironment::new(
            None,
            Some(state_base.path().as_os_str().to_owned()),
            None,
        )))
        .unwrap();

        assert!(listing.session_ids().is_empty());
        assert!(!listing.truncated());
        assert!(!state_base.path().join(crate::STATE_NAMESPACE).exists());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn existing_safe_store_returns_lifecycle_written_ids() {
        let state_base = TempDirectory::new();
        let state_root = create_xdg_state_root(state_base.path(), 0o700);
        let store = FileSessionStore::open(&state_root).unwrap();
        for (id, incarnation) in [("beta", "inc-beta"), ("alpha", "inc-alpha")] {
            let record = SessionRecord::empty(
                SessionId::new(id).unwrap(),
                SessionIncarnationId::new(incarnation).unwrap(),
            );
            block_on(store.save(record, None)).unwrap();
        }

        let listing = block_on(list_native_sessions(NativeEnvironment::new(
            None,
            Some(state_base.path().as_os_str().to_owned()),
            None,
        )))
        .unwrap();
        let ids = listing
            .session_ids()
            .iter()
            .map(SessionId::as_str)
            .collect::<Vec<_>>();
        assert_eq!(ids, ["alpha", "beta"]);
        assert!(!listing.truncated());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn permissive_final_state_root_is_rejected_as_unsafe() {
        let state_base = TempDirectory::new();
        create_xdg_state_root(state_base.path(), 0o755);

        let error = block_on(list_native_sessions(NativeEnvironment::new(
            None,
            Some(state_base.path().as_os_str().to_owned()),
            None,
        )))
        .unwrap_err();
        assert_eq!(error.kind(), NativeSessionListingErrorKind::UnsafeStateRoot);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn non_directory_final_state_roots_are_rejected_as_unsafe() {
        let file_base = TempDirectory::new();
        fs::write(
            file_base.path().join(crate::STATE_NAMESPACE),
            b"not a directory",
        )
        .unwrap();
        let file_error = block_on(list_native_sessions(NativeEnvironment::new(
            None,
            Some(file_base.path().as_os_str().to_owned()),
            None,
        )))
        .unwrap_err();

        let symlink_base = TempDirectory::new();
        let target = symlink_base.path().join("target");
        fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, symlink_base.path().join(crate::STATE_NAMESPACE))
            .unwrap();
        let symlink_error = block_on(list_native_sessions(NativeEnvironment::new(
            None,
            Some(symlink_base.path().as_os_str().to_owned()),
            None,
        )))
        .unwrap_err();

        assert_eq!(
            file_error.kind(),
            NativeSessionListingErrorKind::UnsafeStateRoot
        );
        assert_eq!(
            symlink_error.kind(),
            NativeSessionListingErrorKind::UnsafeStateRoot
        );
    }
}
