use crate::{
    BoxFuture, BuildError, EngineError, EventSink, ModelProvider, NoopEventSink, PermissionHandler,
    Session, SessionId, SessionRecord, SessionStore, Tool, ToolName, ToolSpec,
};
use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, Weak};

#[cfg(test)]
use std::sync::Barrier;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

/// Builder requiring explicit authority-bearing components.
#[derive(Default)]
pub struct EngineBuilder {
    provider: Option<Arc<dyn ModelProvider>>,
    session_store: Option<Arc<dyn SessionStore>>,
    permission_handler: Option<Arc<dyn PermissionHandler>>,
    event_sink: Option<Arc<dyn EventSink>>,
    tools: BTreeMap<ToolName, RegisteredTool>,
    duplicate_tool: Option<ToolName>,
    limits: EngineLimits,
}

/// Nonzero resource bounds enforced by every turn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EngineLimits {
    pub max_model_rounds: NonZeroUsize,
    pub max_tool_calls_per_turn: NonZeroUsize,
    pub max_tool_calls_per_round: NonZeroUsize,
    pub max_assistant_text_bytes: NonZeroUsize,
    pub max_reasoning_bytes: NonZeroUsize,
    pub max_tool_argument_bytes: NonZeroUsize,
    pub max_serialized_tool_result_bytes: NonZeroUsize,
    pub max_cumulative_tool_result_bytes: NonZeroUsize,
}

impl Default for EngineLimits {
    fn default() -> Self {
        Self {
            max_model_rounds: NonZeroUsize::new(8).expect("default is nonzero"),
            max_tool_calls_per_turn: NonZeroUsize::new(16).expect("default is nonzero"),
            max_tool_calls_per_round: NonZeroUsize::new(4).expect("default is nonzero"),
            max_assistant_text_bytes: NonZeroUsize::new(1024 * 1024).expect("default is nonzero"),
            max_reasoning_bytes: NonZeroUsize::new(1024 * 1024).expect("default is nonzero"),
            max_tool_argument_bytes: NonZeroUsize::new(64 * 1024).expect("default is nonzero"),
            max_serialized_tool_result_bytes: NonZeroUsize::new(64 * 1024)
                .expect("default is nonzero"),
            max_cumulative_tool_result_bytes: NonZeroUsize::new(256 * 1024)
                .expect("default is nonzero"),
        }
    }
}

impl fmt::Debug for EngineBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EngineBuilder")
            .field("has_provider", &self.provider.is_some())
            .field("has_session_store", &self.session_store.is_some())
            .field("has_permission_handler", &self.permission_handler.is_some())
            .field("tool_count", &self.tools.len())
            .finish_non_exhaustive()
    }
}

impl EngineBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn provider(mut self, provider: impl ModelProvider) -> Self {
        self.provider = Some(Arc::new(provider));
        self
    }

    #[must_use]
    pub fn shared_provider(mut self, provider: Arc<dyn ModelProvider>) -> Self {
        self.provider = Some(provider);
        self
    }

    #[must_use]
    pub fn session_store(mut self, store: impl SessionStore) -> Self {
        self.session_store = Some(Arc::new(store));
        self
    }

    #[must_use]
    pub fn shared_session_store(mut self, store: Arc<dyn SessionStore>) -> Self {
        self.session_store = Some(store);
        self
    }

    #[must_use]
    pub fn permission_handler(mut self, handler: impl PermissionHandler) -> Self {
        self.permission_handler = Some(Arc::new(handler));
        self
    }

    #[must_use]
    pub fn shared_permission_handler(mut self, handler: Arc<dyn PermissionHandler>) -> Self {
        self.permission_handler = Some(handler);
        self
    }

    #[must_use]
    pub fn event_sink(mut self, sink: impl EventSink) -> Self {
        self.event_sink = Some(Arc::new(sink));
        self
    }

    #[must_use]
    pub fn shared_event_sink(mut self, sink: Arc<dyn EventSink>) -> Self {
        self.event_sink = Some(sink);
        self
    }

    /// Replaces the conservative default per-turn resource bounds.
    #[must_use]
    pub fn limits(mut self, limits: EngineLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Registers a tool. Duplicate names make [`Self::build`] fail closed.
    #[must_use]
    pub fn tool(mut self, tool: impl Tool) -> Self {
        let tool = Arc::new(tool);
        let spec = tool.spec();
        let name = spec.name.clone();
        if self
            .tools
            .insert(name.clone(), RegisteredTool { spec, tool })
            .is_some()
        {
            self.duplicate_tool = Some(name);
        }
        self
    }

    #[must_use]
    pub fn shared_tool(mut self, tool: Arc<dyn Tool>) -> Self {
        let spec = tool.spec();
        let name = spec.name.clone();
        if self
            .tools
            .insert(name.clone(), RegisteredTool { spec, tool })
            .is_some()
        {
            self.duplicate_tool = Some(name);
        }
        self
    }

    /// Constructs the engine after validating its explicit dependencies.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError`] when a required component is absent or when two
    /// registered tools have the same name.
    pub fn build(self) -> Result<Engine, BuildError> {
        if let Some(name) = self.duplicate_tool {
            return Err(BuildError::DuplicateTool(name.to_string()));
        }
        let provider = self.provider.ok_or(BuildError::MissingProvider)?;
        let session_store = self.session_store.ok_or(BuildError::MissingSessionStore)?;
        let permission_handler = self
            .permission_handler
            .ok_or(BuildError::MissingPermissionHandler)?;
        let tool_specs = self
            .tools
            .values()
            .map(|registered| registered.spec.clone())
            .collect();
        Ok(Engine {
            inner: Arc::new(EngineInner {
                provider,
                session_store,
                permission_handler,
                event_sink: self.event_sink.unwrap_or_else(|| Arc::new(NoopEventSink)),
                tools: self.tools,
                tool_specs,
                limits: self.limits,
                sessions: Arc::new(SessionRegistry::default()),
            }),
        })
    }
}

/// Configured provider-neutral engine.
#[derive(Clone)]
pub struct Engine {
    pub(crate) inner: Arc<EngineInner>,
}

pub(crate) struct EngineInner {
    pub(crate) provider: Arc<dyn ModelProvider>,
    pub(crate) session_store: Arc<dyn SessionStore>,
    pub(crate) permission_handler: Arc<dyn PermissionHandler>,
    pub(crate) event_sink: Arc<dyn EventSink>,
    tools: BTreeMap<ToolName, RegisteredTool>,
    tool_specs: Vec<ToolSpec>,
    pub(crate) limits: EngineLimits,
    sessions: Arc<SessionRegistry>,
}

#[derive(Default)]
pub(crate) struct SessionRegistry {
    entries: Mutex<BTreeMap<SessionId, Weak<crate::session::SessionState>>>,
    #[cfg(test)]
    entry_checks: AtomicUsize,
    #[cfg(test)]
    before_remove: Mutex<Option<Arc<Barrier>>>,
}

pub(crate) struct SessionRegistration {
    registry: Weak<SessionRegistry>,
    id: SessionId,
    state: Weak<crate::session::SessionState>,
}

struct RegisteredTool {
    spec: ToolSpec,
    tool: Arc<dyn Tool>,
}

impl EngineInner {
    pub(crate) fn tool_specs(&self) -> Vec<ToolSpec> {
        self.tool_specs.clone()
    }

    pub(crate) fn tool(&self, name: &ToolName) -> Option<Arc<dyn Tool>> {
        self.tools
            .get(name)
            .map(|registered| Arc::clone(&registered.tool))
    }

    fn session_state(
        &self,
        record: SessionRecord,
        persisted: bool,
    ) -> Arc<crate::session::SessionState> {
        self.sessions.session_state(record, persisted)
    }
}

impl SessionRegistry {
    fn session_state(
        self: &Arc<Self>,
        record: SessionRecord,
        persisted: bool,
    ) -> Arc<crate::session::SessionState> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        #[cfg(test)]
        self.entry_checks.fetch_add(1, Ordering::Relaxed);
        if let Some(state) = entries.get(&record.id).and_then(Weak::upgrade) {
            return state;
        }
        let id = record.id.clone();
        let state =
            crate::session::SessionState::new_registered(record, persisted, Arc::downgrade(self));
        entries.insert(id, Arc::downgrade(&state));
        state
    }

    fn remove_if_matches(&self, id: &SessionId, state: &Weak<crate::session::SessionState>) {
        #[cfg(test)]
        if let Some(barrier) = self
            .before_remove
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            barrier.wait();
            barrier.wait();
        }
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if entries
            .get(id)
            .is_some_and(|registered| Weak::ptr_eq(registered, state))
        {
            entries.remove(id);
        }
    }
}

impl SessionRegistration {
    pub(crate) fn new(
        registry: Weak<SessionRegistry>,
        id: SessionId,
        state: Weak<crate::session::SessionState>,
    ) -> Self {
        Self {
            registry,
            id,
            state,
        }
    }
}

impl Drop for SessionRegistration {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            registry.remove_if_matches(&self.id, &self.state);
        }
    }
}

impl fmt::Debug for Engine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Engine")
            .field("provider", &self.inner.provider.name())
            .field("tool_count", &self.inner.tools.len())
            .finish_non_exhaustive()
    }
}

impl Engine {
    #[must_use]
    pub fn builder() -> EngineBuilder {
        EngineBuilder::new()
    }

    /// Returns the immutable resource bounds used by this engine.
    #[must_use]
    pub fn limits(&self) -> EngineLimits {
        self.inner.limits
    }

    /// Returns the engine-canonical in-memory handle for `id`.
    ///
    /// If this engine already loaded or created the session, the returned
    /// handle shares that state and its live-turn lease rather than replacing
    /// it. Durable state is reconciled by [`Self::load_session`] and prompt
    /// reservation.
    #[must_use]
    pub fn create_session(&self, id: SessionId) -> Session {
        let state = self.inner.session_state(SessionRecord::empty(id), false);
        Session::from_state(Arc::clone(&self.inner), state)
    }

    /// Loads a stored session without blocking an executor thread.
    #[must_use]
    pub fn load_session(
        &self,
        id: SessionId,
    ) -> BoxFuture<'static, Result<Option<Session>, EngineError>> {
        let inner = Arc::clone(&self.inner);
        Box::pin(async move {
            let record = inner.session_store.load(id.clone()).await?;
            if let Some(record) = &record
                && record.id != id
            {
                return Err(EngineError::Protocol(format!(
                    "session store returned ID {} for requested ID {id}",
                    record.id
                )));
            }
            if let Some(record) = &record {
                crate::session::SessionState::validate_loaded(record)?;
            }
            record
                .map(|record| {
                    let state = inner.session_state(record.clone(), true);
                    state.reconcile_loaded(record)?;
                    Ok(Session::from_state(Arc::clone(&inner), state))
                })
                .transpose()
        })
    }

    #[must_use]
    pub fn provider(&self) -> &dyn ModelProvider {
        self.inner.provider.as_ref()
    }

    #[must_use]
    pub fn session_store(&self) -> &dyn SessionStore {
        self.inner.session_store.as_ref()
    }

    #[must_use]
    pub fn permission_handler(&self) -> &dyn PermissionHandler {
        self.inner.permission_handler.as_ref()
    }

    #[must_use]
    pub fn event_sink(&self) -> &dyn EventSink {
        self.inner.event_sink.as_ref()
    }

    #[must_use]
    pub fn tool(&self, name: &ToolName) -> Option<&dyn Tool> {
        self.inner.tools.get(name).map(|tool| tool.tool.as_ref())
    }

    #[must_use]
    pub fn tool_specs(&self) -> Vec<ToolSpec> {
        self.inner.tool_specs()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        BoxFuture, CancellationToken, ModelEventStream, ModelProvider, ModelRequest,
        PermissionDecision, PermissionError, PermissionHandler, PermissionRequest, ProviderError,
        SessionId, SessionRecord, SessionRevision, SessionStore, SessionStoreError,
    };
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Barrier};

    #[derive(Debug)]
    struct UnusedProvider;

    impl ModelProvider for UnusedProvider {
        fn name(&self) -> &'static str {
            "unused"
        }

        fn stream(
            &self,
            _request: ModelRequest,
            _cancellation: CancellationToken,
        ) -> BoxFuture<'_, Result<ModelEventStream, ProviderError>> {
            Box::pin(async { unreachable!("turn stream is not polled") })
        }
    }

    #[derive(Clone, Debug)]
    struct CorruptStore(SessionRecord);

    impl SessionStore for CorruptStore {
        fn load(
            &self,
            _id: SessionId,
        ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
            let record = self.0.clone();
            Box::pin(async move { Ok(Some(record)) })
        }

        fn save(
            &self,
            _record: SessionRecord,
            _expected_revision: Option<SessionRevision>,
        ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
            Box::pin(async { Ok(SessionRevision(1)) })
        }
    }

    #[derive(Debug)]
    struct DenyPermissions;

    impl PermissionHandler for DenyPermissions {
        fn authorize(
            &self,
            _request: PermissionRequest,
        ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>> {
            Box::pin(async {
                Ok(PermissionDecision::Deny {
                    reason: "unused".to_owned(),
                })
            })
        }
    }

    fn test_engine() -> super::Engine {
        super::Engine::builder()
            .provider(UnusedProvider)
            .session_store(CorruptStore(SessionRecord::empty(
                SessionId::new("unused-store-record").unwrap(),
            )))
            .permission_handler(DenyPermissions)
            .build()
            .unwrap()
    }

    #[test]
    fn many_live_sessions_use_one_targeted_registry_check_per_request() {
        const SESSION_COUNT: usize = 4_096;

        let engine = test_engine();
        let mut live = Vec::with_capacity(SESSION_COUNT);
        for index in 0..SESSION_COUNT {
            let id = SessionId::new(format!("scaling-{index:04}")).unwrap();
            live.push(engine.create_session(id));
        }
        assert_eq!(
            engine.inner.sessions.entry_checks.load(Ordering::Relaxed),
            SESSION_COUNT
        );
        assert_eq!(
            engine.inner.sessions.entries.lock().unwrap().len(),
            SESSION_COUNT
        );

        for session in &live {
            drop(engine.create_session(session.id()));
        }
        assert_eq!(
            engine.inner.sessions.entry_checks.load(Ordering::Relaxed),
            SESSION_COUNT * 2
        );
        assert_eq!(
            engine.inner.sessions.entries.lock().unwrap().len(),
            SESSION_COUNT
        );
    }

    #[test]
    fn dropping_the_last_session_handle_reclaims_its_registry_key() {
        let engine = test_engine();
        let id = SessionId::new("drop-reclaims-key").unwrap();
        let session = engine.create_session(id.clone());
        assert!(
            engine
                .inner
                .sessions
                .entries
                .lock()
                .unwrap()
                .contains_key(&id)
        );

        drop(session);

        assert!(
            !engine
                .inner
                .sessions
                .entries
                .lock()
                .unwrap()
                .contains_key(&id)
        );
    }

    #[test]
    fn delayed_old_state_drop_cannot_remove_a_concurrent_replacement() {
        let engine = test_engine();
        let registry = Arc::clone(&engine.inner.sessions);
        let id = SessionId::new("replacement-race").unwrap();
        let original = engine.create_session(id.clone());
        let barrier = Arc::new(Barrier::new(2));
        *registry.before_remove.lock().unwrap() = Some(Arc::clone(&barrier));

        let dropping = std::thread::spawn(move || drop(original));
        barrier.wait();
        let replacement = engine.create_session(id.clone());
        barrier.wait();
        dropping.join().unwrap();
        *registry.before_remove.lock().unwrap() = None;

        let registered = registry.entries.lock().unwrap().get(&id).cloned().unwrap();
        assert_eq!(registered.strong_count(), 1);
        let converged = engine.create_session(id.clone());
        assert_eq!(registered.strong_count(), 2);

        drop(replacement);
        assert!(registry.entries.lock().unwrap().contains_key(&id));
        drop(converged);
        assert!(!registry.entries.lock().unwrap().contains_key(&id));
    }

    #[test]
    fn corrupt_load_is_rejected_before_registry_publication() {
        let id = SessionId::new("reject-before-publication").unwrap();
        let mut corrupt = SessionRecord::empty(id.clone());
        corrupt.revision = SessionRevision(9);
        corrupt.next_turn_sequence = 0;
        let engine = super::Engine::builder()
            .provider(UnusedProvider)
            .session_store(CorruptStore(corrupt))
            .permission_handler(DenyPermissions)
            .build()
            .unwrap();

        assert!(futures_executor::block_on(engine.load_session(id.clone())).is_err());
        assert!(engine.inner.sessions.entries.lock().unwrap().is_empty());

        let created = engine.create_session(id);
        assert_eq!(created.record().next_turn_sequence, 1);
        let turn = futures_executor::block_on(created.prompt("safe first turn")).unwrap();
        assert_eq!(turn.id().as_str(), "turn-1");
    }
}
