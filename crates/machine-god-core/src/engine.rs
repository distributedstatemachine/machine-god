use crate::{
    BoxFuture, BuildError, EngineError, EventSink, ModelProvider, NoopEventSink, PermissionHandler,
    Session, SessionId, SessionRecord, SessionStore, Tool, ToolName, ToolSpec,
};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// Builder requiring explicit authority-bearing components.
#[derive(Default)]
pub struct EngineBuilder {
    provider: Option<Arc<dyn ModelProvider>>,
    session_store: Option<Arc<dyn SessionStore>>,
    permission_handler: Option<Arc<dyn PermissionHandler>>,
    event_sink: Option<Arc<dyn EventSink>>,
    tools: BTreeMap<ToolName, RegisteredTool>,
    duplicate_tool: Option<ToolName>,
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
        Ok(Engine {
            inner: Arc::new(EngineInner {
                provider,
                session_store,
                permission_handler,
                event_sink: self.event_sink.unwrap_or_else(|| Arc::new(NoopEventSink)),
                tools: self.tools,
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
}

struct RegisteredTool {
    spec: ToolSpec,
    tool: Arc<dyn Tool>,
}

impl EngineInner {
    pub(crate) fn tool_specs(&self) -> Vec<ToolSpec> {
        self.tools.values().map(|tool| tool.spec.clone()).collect()
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

    /// Creates a new in-memory session handle. Persistence is explicit through
    /// the configured store and later orchestration.
    #[must_use]
    pub fn create_session(&self, id: SessionId) -> Session {
        Session::from_record(Arc::clone(&self.inner), SessionRecord::empty(id))
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
            Ok(record.map(|record| Session::from_record(inner, record)))
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
