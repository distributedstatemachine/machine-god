use std::error::Error;
use std::fmt;
use std::sync::Arc;

use machine_god_core::{
    BoxFuture, Engine, EngineError, Session, SessionId, SessionIncarnationId, SessionRecord,
    SessionStore, SessionStoreError, SessionStoreErrorKind,
};

use crate::FileSessionStore;

/// Maximum number of source values considered while avoiding the current
/// session incarnation during reset.
pub const MAX_SESSION_INCARNATION_ATTEMPTS: usize = 8;

const RANDOM_INCARNATION_BYTES: usize = 32;
const INCARNATION_PREFIX: &str = "inc-";
const LOWER_HEX: &[u8; 16] = b"0123456789abcdef";

/// Fixed, zero-data failure from an injected session-incarnation source.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct SessionIncarnationSourceError;

impl SessionIncarnationSourceError {
    /// Creates a fixed source failure without retaining source diagnostics.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for SessionIncarnationSourceError {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for SessionIncarnationSourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionIncarnationSourceError")
            .finish()
    }
}

impl fmt::Display for SessionIncarnationSourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("session incarnation source failed")
    }
}

impl Error for SessionIncarnationSourceError {}

/// Stable category for native session-lifecycle construction failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeSessionLifecycleBuildErrorKind {
    /// The engine is configured with a different session-store allocation.
    MismatchedSessionStore,
}

impl NativeSessionLifecycleBuildErrorKind {
    /// Returns the stable, machine-readable name of this category.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MismatchedSessionStore => "mismatched_session_store",
        }
    }
}

/// Fixed, redacted native session-lifecycle construction failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct NativeSessionLifecycleBuildError {
    kind: NativeSessionLifecycleBuildErrorKind,
}

impl NativeSessionLifecycleBuildError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> NativeSessionLifecycleBuildErrorKind {
        self.kind
    }

    const fn new(kind: NativeSessionLifecycleBuildErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for NativeSessionLifecycleBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeSessionLifecycleBuildError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for NativeSessionLifecycleBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            NativeSessionLifecycleBuildErrorKind::MismatchedSessionStore => {
                "native session lifecycle store does not match engine"
            }
        })
    }
}

impl Error for NativeSessionLifecycleBuildError {}

/// Synchronous, bounded source of globally unique session-incarnation IDs.
///
/// Implementations must not derive identifiers from session content or secret
/// configuration. Calls occur only after a lifecycle future is polled.
pub trait SessionIncarnationSource: Send + Sync + 'static {
    /// Returns the next validated incarnation ID.
    ///
    /// # Errors
    ///
    /// Returns a fixed error when an identifier cannot be supplied.
    fn next_incarnation_id(&self) -> Result<SessionIncarnationId, SessionIncarnationSourceError>;
}

#[derive(Debug)]
struct OsSessionIncarnationSource;

impl SessionIncarnationSource for OsSessionIncarnationSource {
    fn next_incarnation_id(&self) -> Result<SessionIncarnationId, SessionIncarnationSourceError> {
        let mut random = [0_u8; RANDOM_INCARNATION_BYTES];
        getrandom::fill(&mut random).map_err(|_| SessionIncarnationSourceError::new())?;
        let mut value = String::with_capacity(INCARNATION_PREFIX.len() + random.len() * 2);
        value.push_str(INCARNATION_PREFIX);
        for byte in random {
            value.push(char::from(LOWER_HEX[usize::from(byte >> 4)]));
            value.push(char::from(LOWER_HEX[usize::from(byte & 0x0f)]));
        }
        SessionIncarnationId::new(value).map_err(|_| SessionIncarnationSourceError::new())
    }
}

/// Stable category for native session-lifecycle failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeSessionLifecycleErrorKind {
    /// Atomic creation found an existing durable record.
    AlreadyExists,
    /// No durable record exists for the supplied ID.
    NotFound,
    /// The engine already retains an incompatible live session.
    LiveSession,
    /// The incarnation source failed to provide a usable distinct ID.
    IncarnationSource,
    /// An exact observed incarnation or revision no longer matches.
    Conflict,
    /// The current-schema durable record is corrupt.
    Corrupt,
    /// A native persistence operation is unavailable or ambiguous.
    Unavailable,
    /// Engine validation or an internal lifecycle invariant failed.
    Engine,
}

impl NativeSessionLifecycleErrorKind {
    /// Returns the stable, machine-readable name of this category.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AlreadyExists => "already_exists",
            Self::NotFound => "not_found",
            Self::LiveSession => "live_session",
            Self::IncarnationSource => "incarnation_source",
            Self::Conflict => "conflict",
            Self::Corrupt => "corrupt",
            Self::Unavailable => "unavailable",
            Self::Engine => "engine",
        }
    }
}

/// Fixed, redacted native session-lifecycle failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct NativeSessionLifecycleError {
    kind: NativeSessionLifecycleErrorKind,
}

impl NativeSessionLifecycleError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> NativeSessionLifecycleErrorKind {
        self.kind
    }

    const fn new(kind: NativeSessionLifecycleErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for NativeSessionLifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeSessionLifecycleError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for NativeSessionLifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            NativeSessionLifecycleErrorKind::AlreadyExists => "native session already exists",
            NativeSessionLifecycleErrorKind::NotFound => "native session was not found",
            NativeSessionLifecycleErrorKind::LiveSession => {
                "native session is incompatible with a live session"
            }
            NativeSessionLifecycleErrorKind::IncarnationSource => {
                "native session incarnation source failed"
            }
            NativeSessionLifecycleErrorKind::Conflict => "native session changed concurrently",
            NativeSessionLifecycleErrorKind::Corrupt => "native session record is corrupt",
            NativeSessionLifecycleErrorKind::Unavailable => {
                "native session persistence is unavailable"
            }
            NativeSessionLifecycleErrorKind::Engine => "native session engine failed",
        })
    }
}

impl Error for NativeSessionLifecycleError {}

/// By-ID durable lifecycle over one engine and its exact concrete file store.
///
/// Construction verifies that `engine` contains the exact supplied
/// [`FileSessionStore`] allocation. [`crate::NativeReferenceHost`] guarantees
/// that identity. Every operation is inert until its returned future is first
/// polled; filesystem work and OS randomness may block that polling thread.
#[derive(Clone)]
pub struct NativeSessionLifecycle {
    engine: Engine,
    session_store: Arc<FileSessionStore>,
    incarnation_source: Arc<dyn SessionIncarnationSource>,
}

impl NativeSessionLifecycle {
    /// Constructs a lifecycle using 256 bits of OS randomness per incarnation.
    ///
    /// # Errors
    ///
    /// Returns a fixed error unless `session_store` is the exact allocation
    /// configured into `engine`.
    pub fn new(
        engine: Engine,
        session_store: Arc<FileSessionStore>,
    ) -> Result<Self, NativeSessionLifecycleBuildError> {
        Self::with_incarnation_source(engine, session_store, OsSessionIncarnationSource)
    }

    /// Constructs a lifecycle with an owned deterministic or custom source.
    ///
    /// # Errors
    ///
    /// Returns a fixed error unless `session_store` is the exact allocation
    /// configured into `engine`.
    pub fn with_incarnation_source(
        engine: Engine,
        session_store: Arc<FileSessionStore>,
        incarnation_source: impl SessionIncarnationSource,
    ) -> Result<Self, NativeSessionLifecycleBuildError> {
        Self::shared_incarnation_source(engine, session_store, Arc::new(incarnation_source))
    }

    /// Constructs a lifecycle with a shared deterministic or custom source.
    ///
    /// # Errors
    ///
    /// Returns a fixed error unless `session_store` is the exact allocation
    /// configured into `engine`.
    pub fn shared_incarnation_source(
        engine: Engine,
        session_store: Arc<FileSessionStore>,
        incarnation_source: Arc<dyn SessionIncarnationSource>,
    ) -> Result<Self, NativeSessionLifecycleBuildError> {
        let supplied_store: &dyn SessionStore = session_store.as_ref();
        if !std::ptr::eq(engine.session_store(), supplied_store) {
            return Err(NativeSessionLifecycleBuildError::new(
                NativeSessionLifecycleBuildErrorKind::MismatchedSessionStore,
            ));
        }
        Ok(Self {
            engine,
            session_store,
            incarnation_source,
        })
    }

    /// Returns the exact engine used for resume and canonical session handles.
    #[must_use]
    pub const fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Returns the exact concrete file-store allocation used for persistence.
    #[must_use]
    pub const fn session_store(&self) -> &Arc<FileSessionStore> {
        &self.session_store
    }

    /// Atomically creates and persists an empty current-schema session.
    #[must_use]
    pub fn create(
        &self,
        id: SessionId,
    ) -> BoxFuture<'static, Result<Session, NativeSessionLifecycleError>> {
        let lifecycle = self.clone();
        Box::pin(async move { lifecycle.create_polled(id).await })
    }

    /// Resumes the engine-canonical handle for one durable session.
    #[must_use]
    pub fn resume(
        &self,
        id: SessionId,
    ) -> BoxFuture<'static, Result<Session, NativeSessionLifecycleError>> {
        let lifecycle = self.clone();
        Box::pin(async move { lifecycle.resume_polled(id).await })
    }

    /// Replays one validated current-schema durable record as an owned snapshot.
    #[must_use]
    pub fn replay(
        &self,
        id: SessionId,
    ) -> BoxFuture<'static, Result<SessionRecord, NativeSessionLifecycleError>> {
        let lifecycle = self.clone();
        Box::pin(async move { lifecycle.replay_polled(id).await })
    }

    /// Atomically replaces one durable session with a new empty incarnation.
    #[must_use]
    pub fn reset(
        &self,
        id: SessionId,
    ) -> BoxFuture<'static, Result<Session, NativeSessionLifecycleError>> {
        let lifecycle = self.clone();
        Box::pin(async move { lifecycle.reset_polled(id).await })
    }

    async fn create_polled(&self, id: SessionId) -> Result<Session, NativeSessionLifecycleError> {
        if self.load_record(id.clone()).await?.is_some() {
            return Err(NativeSessionLifecycleError::new(
                NativeSessionLifecycleErrorKind::AlreadyExists,
            ));
        }
        let incarnation_id = self.next_incarnation_id()?;
        let candidate = SessionRecord::empty(id.clone(), incarnation_id);
        let reservation = self.reserve_candidate(&candidate)?;
        match self.session_store.create_empty_record(candidate) {
            Ok(_) => {}
            Err(error) if error.kind == SessionStoreErrorKind::Conflict => {
                return Err(NativeSessionLifecycleError::new(
                    NativeSessionLifecycleErrorKind::AlreadyExists,
                ));
            }
            Err(error) => return Err(map_store_error(error)),
        }
        let loaded = self.load_canonical(id).await?;
        drop(reservation);
        Ok(loaded)
    }

    async fn resume_polled(&self, id: SessionId) -> Result<Session, NativeSessionLifecycleError> {
        self.load_canonical(id).await
    }

    async fn replay_polled(
        &self,
        id: SessionId,
    ) -> Result<SessionRecord, NativeSessionLifecycleError> {
        self.load_record(id).await?.ok_or_else(|| {
            NativeSessionLifecycleError::new(NativeSessionLifecycleErrorKind::NotFound)
        })
    }

    async fn reset_polled(&self, id: SessionId) -> Result<Session, NativeSessionLifecycleError> {
        let observed = self.load_record(id.clone()).await?.ok_or_else(|| {
            NativeSessionLifecycleError::new(NativeSessionLifecycleErrorKind::NotFound)
        })?;
        let incarnation_id = self.next_distinct_incarnation_id(&observed.incarnation_id)?;
        let replacement = SessionRecord::empty(id.clone(), incarnation_id);
        let reservation = self.reserve_candidate(&replacement)?;
        match self.session_store.reset_record(&observed, replacement) {
            Ok(_) => {}
            Err(error) if error.kind == SessionStoreErrorKind::Conflict => {
                return Err(NativeSessionLifecycleError::new(
                    NativeSessionLifecycleErrorKind::Conflict,
                ));
            }
            Err(error) => return Err(map_store_error(error)),
        }
        let loaded = self.load_canonical(id).await?;
        drop(reservation);
        Ok(loaded)
    }

    async fn load_record(
        &self,
        id: SessionId,
    ) -> Result<Option<SessionRecord>, NativeSessionLifecycleError> {
        self.session_store.load(id).await.map_err(map_store_error)
    }

    async fn load_canonical(&self, id: SessionId) -> Result<Session, NativeSessionLifecycleError> {
        self.engine
            .load_session(id)
            .await
            .map_err(map_engine_error)?
            .ok_or_else(|| {
                NativeSessionLifecycleError::new(NativeSessionLifecycleErrorKind::NotFound)
            })
    }

    fn reserve_candidate(
        &self,
        candidate: &SessionRecord,
    ) -> Result<Session, NativeSessionLifecycleError> {
        let session = self
            .engine
            .create_session(candidate.id.clone(), candidate.incarnation_id.clone())
            .map_err(map_engine_error)?;
        if session.has_active_turn() || session.record() != *candidate {
            return Err(NativeSessionLifecycleError::new(
                NativeSessionLifecycleErrorKind::LiveSession,
            ));
        }
        Ok(session)
    }

    fn next_incarnation_id(&self) -> Result<SessionIncarnationId, NativeSessionLifecycleError> {
        self.incarnation_source.next_incarnation_id().map_err(|_| {
            NativeSessionLifecycleError::new(NativeSessionLifecycleErrorKind::IncarnationSource)
        })
    }

    fn next_distinct_incarnation_id(
        &self,
        current: &SessionIncarnationId,
    ) -> Result<SessionIncarnationId, NativeSessionLifecycleError> {
        for _ in 0..MAX_SESSION_INCARNATION_ATTEMPTS {
            let candidate = self.next_incarnation_id()?;
            if &candidate != current {
                return Ok(candidate);
            }
        }
        Err(NativeSessionLifecycleError::new(
            NativeSessionLifecycleErrorKind::IncarnationSource,
        ))
    }
}

impl fmt::Debug for NativeSessionLifecycle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeSessionLifecycle")
            .field("has_engine", &true)
            .field("has_session_store", &true)
            .field("has_incarnation_source", &true)
            .finish_non_exhaustive()
    }
}

fn map_engine_error(error: EngineError) -> NativeSessionLifecycleError {
    let kind = match error {
        EngineError::SessionIncarnationConflict | EngineError::SessionBusy => {
            NativeSessionLifecycleErrorKind::LiveSession
        }
        EngineError::Store(error) => return map_store_error(error),
        EngineError::Provider(_)
        | EngineError::Permission(_)
        | EngineError::EventSink(_)
        | EngineError::Protocol(_)
        | _ => NativeSessionLifecycleErrorKind::Engine,
    };
    NativeSessionLifecycleError::new(kind)
}

fn map_store_error(error: SessionStoreError) -> NativeSessionLifecycleError {
    let kind = match error.kind {
        SessionStoreErrorKind::NotFound => NativeSessionLifecycleErrorKind::NotFound,
        SessionStoreErrorKind::Conflict => NativeSessionLifecycleErrorKind::Conflict,
        SessionStoreErrorKind::Corrupt => NativeSessionLifecycleErrorKind::Corrupt,
        SessionStoreErrorKind::Unavailable => NativeSessionLifecycleErrorKind::Unavailable,
        SessionStoreErrorKind::Other | _ => NativeSessionLifecycleErrorKind::Engine,
    };
    drop(error);
    NativeSessionLifecycleError::new(kind)
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use machine_god_testkit::{ScriptedModelProvider, ScriptedPermissionHandler};

    use super::*;

    #[derive(Debug)]
    struct CountingSource(Arc<AtomicUsize>);

    impl SessionIncarnationSource for CountingSource {
        fn next_incarnation_id(
            &self,
        ) -> Result<SessionIncarnationId, SessionIncarnationSourceError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            SessionIncarnationId::new("inc-test-value")
                .map_err(|_| SessionIncarnationSourceError::new())
        }
    }

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new(label: &str) -> Self {
            for _ in 0..MAX_SESSION_INCARNATION_ATTEMPTS {
                let mut random = [0_u8; 16];
                getrandom::fill(&mut random).expect("test temporary-name randomness");
                let mut suffix = String::with_capacity(random.len() * 2);
                for byte in random {
                    write!(&mut suffix, "{byte:02x}").expect("write test path suffix");
                }
                let path = std::env::temp_dir().join(format!(
                    "machine-god-session-lifecycle-{label}-{}-{suffix}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("create test directory: {error}"),
                }
            }
            panic!("allocate unique test directory");
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn mismatched_store_is_rejected_before_source_or_filesystem_mutation() {
        let engine_root = TempDirectory::new("engine");
        let supplied_root = TempDirectory::new("supplied");
        let engine_store = Arc::new(FileSessionStore::open(engine_root.path()).unwrap());
        let supplied_store = Arc::new(FileSessionStore::open(supplied_root.path()).unwrap());
        let shared_engine_store: Arc<dyn SessionStore> = engine_store;
        let engine = Engine::builder()
            .provider(ScriptedModelProvider::new("test", []))
            .shared_session_store(shared_engine_store)
            .permission_handler(ScriptedPermissionHandler::new([]))
            .build()
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));

        let error = NativeSessionLifecycle::with_incarnation_source(
            engine,
            supplied_store,
            CountingSource(Arc::clone(&calls)),
        )
        .unwrap_err();

        assert_eq!(
            error.kind(),
            NativeSessionLifecycleBuildErrorKind::MismatchedSessionStore
        );
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert_eq!(fs::read_dir(engine_root.path()).unwrap().count(), 0);
        assert_eq!(fs::read_dir(supplied_root.path()).unwrap().count(), 0);
    }
}
