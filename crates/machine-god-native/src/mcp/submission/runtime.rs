use super::{
    CancellationToken, MAX_MCP_SUBMISSION_BINDING_BYTES, McpSubmissionError, Result, ToolName,
    next_generation,
};
use std::fmt;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// Immutable native runtime identity. Bytes are identity evidence, not parsed
/// configuration, schema validation, credentials authority or executable input.
/// Trusted runtime composition supplies the exact admitted snapshots, including
/// resolved authentication identity; no mutable external object is consulted.
pub struct McpSubmissionRuntimeBinding {
    server: Arc<str>,
    tool: ToolName,
    remote_tool: Arc<str>,
    configuration: Arc<[u8]>,
    schema: BindingSchema,
    authentication: Arc<[u8]>,
}

enum BindingSchema {
    Bytes(Arc<[u8]>),
    Admitted(crate::mcp::schema::McpSchema),
}
impl BindingSchema {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Bytes(bytes) => bytes,
            Self::Admitted(schema) => schema.raw_json().as_bytes(),
        }
    }
}
impl McpSubmissionRuntimeBinding {
    /// Copies a finite immutable binding. Secret-bearing bytes never appear in
    /// Debug/errors and have no serde representation. This is not secure erasure.
    ///
    /// # Errors
    /// Rejects empty/oversized names and more than 1 MiB aggregate binding bytes.
    pub fn new(
        server: &str,
        tool: ToolName,
        remote_tool: &str,
        configuration: &[u8],
        schema: &[u8],
        authentication: &[u8],
    ) -> Result<Self> {
        validate_binding(
            server,
            &tool,
            remote_tool,
            configuration.len(),
            schema.len(),
            authentication.len(),
        )?;
        Ok(Self {
            server: server.into(),
            tool,
            remote_tool: remote_tool.into(),
            configuration: configuration.into(),
            schema: BindingSchema::Bytes(schema.into()),
            authentication: authentication.into(),
        })
    }
    /// Retains shared immutable server identity and an already admitted schema.
    /// Every logical binding still obeys the legacy aggregate bound; callers
    /// additionally bound distinct shared allocations across the publication.
    /// No configuration, authentication or schema bytes are copied per tool.
    ///
    /// # Errors
    /// Rejects invalid names or more than 1 MiB logical binding bytes.
    pub fn shared(
        server: Arc<str>,
        tool: ToolName,
        remote_tool: Arc<str>,
        configuration: Arc<[u8]>,
        schema: crate::mcp::schema::McpSchema,
        authentication: Arc<[u8]>,
    ) -> Result<Self> {
        validate_binding(
            &server,
            &tool,
            &remote_tool,
            configuration.len(),
            schema.raw_json().len(),
            authentication.len(),
        )?;
        Ok(Self {
            server,
            tool,
            remote_tool,
            configuration,
            schema: BindingSchema::Admitted(schema),
            authentication,
        })
    }
    pub(crate) fn tool_name(&self) -> &ToolName {
        &self.tool
    }
    pub(crate) fn remote_tool(&self) -> &str {
        &self.remote_tool
    }
    pub(crate) fn schema_bytes(&self) -> &[u8] {
        self.schema.bytes()
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn server(&self) -> &str {
        &self.server
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn configuration_bytes(&self) -> &[u8] {
        &self.configuration
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn authentication_bytes(&self) -> &[u8] {
        &self.authentication
    }
}

fn validate_binding(
    server: &str,
    tool: &ToolName,
    remote_tool: &str,
    configuration: usize,
    schema: usize,
    authentication: usize,
) -> Result<()> {
    if server.is_empty() || server.len() > 128 || remote_tool.is_empty() || remote_tool.len() > 256
    {
        return Err(McpSubmissionError::Invalid);
    }
    let total = [
        server.len(),
        tool.as_str().len(),
        remote_tool.len(),
        configuration,
        schema,
        authentication,
    ]
    .into_iter()
    .try_fold(0usize, usize::checked_add)
    .ok_or(McpSubmissionError::Limit)?;
    if total > MAX_MCP_SUBMISSION_BINDING_BYTES {
        Err(McpSubmissionError::Limit)
    } else {
        Ok(())
    }
}

struct State {
    next_generation: Option<u64>,
    active: Option<Arc<McpSubmissionRuntime>>,
}

/// Owner of one executable tool's runtime lineage. Replacement/retirement
/// invalidates all retained old submissions and wakes their queue waiters.
/// Generation exhaustion is terminal: never wraps or reuses a generation.
pub struct McpSubmissionRuntimeOwner {
    state: Mutex<State>,
}
impl Default for McpSubmissionRuntimeOwner {
    fn default() -> Self {
        Self::new()
    }
}
impl McpSubmissionRuntimeOwner {
    /// Creates an inert runtime lineage with no active generation.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                next_generation: Some(1),
                active: None,
            }),
        }
    }
    /// Installs a fully prepared immutable binding and retires the old one.
    /// Failed installation preserves the old active generation.
    ///
    /// # Errors
    /// Rejects poisoned state or exhausted generations.
    pub fn install(
        &self,
        binding: McpSubmissionRuntimeBinding,
    ) -> Result<Arc<McpSubmissionRuntime>> {
        let (runtime, previous) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| McpSubmissionError::Unavailable)?;
            let generation = next_generation(&mut state.next_generation)?;
            let runtime = Arc::new(McpSubmissionRuntime {
                generation,
                binding,
                retired: AtomicBool::new(false),
                cancellation: CancellationToken::new(),
            });
            let previous = state.active.replace(runtime.clone());
            if let Some(previous) = &previous {
                previous.retired.store(true, Ordering::Release);
            }
            (runtime, previous)
        };
        if let Some(previous) = previous {
            previous.cancellation.cancel();
        }
        Ok(runtime)
    }
    /// Invalidates before waking, with no waker/proof destruction under lock.
    pub fn retire(&self) {
        self.retire_deferred().complete();
    }
    /// Marks this lineage invalid without invoking any registered waker. The
    /// caller completes the returned retirement after releasing publication
    /// locks, so a complete catalog can become invalid atomically.
    pub(crate) fn retire_deferred(&self) -> McpDeferredRuntimeRetirement {
        let previous = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let previous = state.active.take();
            if let Some(previous) = &previous {
                previous.retired.store(true, Ordering::Release);
            }
            previous
        };
        McpDeferredRuntimeRetirement(previous)
    }
}

/// Deferred cancellation has no destructor callbacks. Complete outside all
/// publication locks; the allocation is already invalid before completion.
#[must_use]
pub(crate) struct McpDeferredRuntimeRetirement(Option<Arc<McpSubmissionRuntime>>);
impl McpDeferredRuntimeRetirement {
    pub(crate) fn complete(self) {
        if let Some(previous) = self.0 {
            previous.cancellation.cancel();
        }
    }
}
impl Drop for McpSubmissionRuntimeOwner {
    fn drop(&mut self) {
        self.retire();
    }
}

/// Retained allocation identity, not a serializable token. Equal generation
/// numbers from separate owners are deliberately not interchangeable.
pub struct McpSubmissionRuntime {
    generation: u64,
    pub(super) binding: McpSubmissionRuntimeBinding,
    retired: AtomicBool,
    pub(super) cancellation: CancellationToken,
}
impl McpSubmissionRuntime {
    /// Monotonic within its owner; insufficient alone to authorize a request.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    pub(super) fn live(&self) -> Result<()> {
        if self.retired.load(Ordering::Acquire) {
            Err(McpSubmissionError::Unavailable)
        } else {
            Ok(())
        }
    }
}
impl fmt::Debug for McpSubmissionRuntimeBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Read every retained identity component without disclosing its contents.
        let _ = (
            &self.server,
            &self.configuration,
            &self.schema,
            &self.authentication,
        );
        f.write_str("McpSubmissionRuntimeBinding { <redacted> }")
    }
}
impl fmt::Debug for McpSubmissionRuntimeOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpSubmissionRuntimeOwner { <redacted> }")
    }
}
impl fmt::Debug for McpSubmissionRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpSubmissionRuntime { <redacted> }")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_reentrant_waker_test::{Callback, new as reentrant_waker};
    use std::future::Future;
    use std::task::Context;

    fn binding() -> McpSubmissionRuntimeBinding {
        McpSubmissionRuntimeBinding::new(
            "server",
            ToolName::new("tool").unwrap(),
            "remote",
            b"config",
            b"schema",
            b"auth",
        )
        .unwrap()
    }

    #[test]
    fn exhausted_runtime_generation_preserves_last_allocation_without_wraparound() {
        let owner = McpSubmissionRuntimeOwner::new();
        owner.state.lock().unwrap().next_generation = Some(u64::MAX);
        let last = owner.install(binding()).unwrap();
        assert_eq!(last.generation(), u64::MAX);
        assert!(matches!(
            owner.install(binding()),
            Err(McpSubmissionError::GenerationExhausted)
        ));
        assert!(last.live().is_ok());
        assert!(Arc::ptr_eq(
            owner.state.lock().unwrap().active.as_ref().unwrap(),
            &last
        ));
        owner.retire();
        assert!(last.live().is_err());
        assert!(matches!(
            owner.install(binding()),
            Err(McpSubmissionError::GenerationExhausted)
        ));
    }

    #[test]
    fn runtime_replacement_and_retirement_wake_outside_owner_mutex() {
        for replace in [false, true] {
            let owner = Arc::new(McpSubmissionRuntimeOwner::new());
            let runtime = owner.install(binding()).unwrap();
            let observe = owner.clone();
            let (waker, state) = reentrant_waker(Callback::Wake, move || {
                assert!(observe.state.try_lock().is_ok());
            });
            let mut cancelled = Box::pin(runtime.cancellation.cancelled());
            assert!(
                cancelled
                    .as_mut()
                    .poll(&mut Context::from_waker(&waker))
                    .is_pending()
            );
            if replace {
                owner.install(binding()).unwrap();
            } else {
                owner.retire();
            }
            assert_eq!(state.calls(), 1);
            assert!(
                cancelled
                    .as_mut()
                    .poll(&mut Context::from_waker(&waker))
                    .is_ready()
            );
        }
    }

    #[test]
    fn shared_binding_retains_exact_allocations_and_schema_tokens() {
        let configuration: Arc<[u8]> = Arc::from(b"configuration".as_slice());
        let authentication: Arc<[u8]> = Arc::from(b"secret".as_slice());
        let schema = crate::mcp::schema::McpSchema::parse(
            br#"{"type":"object","properties":{"n":{"const":-0}}}"#,
            crate::mcp::schema::McpSchemaLimits::default(),
        )
        .unwrap();
        let binding = McpSubmissionRuntimeBinding::shared(
            Arc::from("server"),
            ToolName::new("tool").unwrap(),
            Arc::from("remote"),
            configuration.clone(),
            schema.clone(),
            authentication.clone(),
        )
        .unwrap();
        assert!(Arc::ptr_eq(&configuration, &binding.configuration));
        assert!(Arc::ptr_eq(&authentication, &binding.authentication));
        assert_eq!(
            binding.schema_bytes().as_ptr(),
            schema.raw_json().as_bytes().as_ptr()
        );
        assert_eq!(binding.schema_bytes(), schema.raw_json().as_bytes());
    }

    #[test]
    fn deferred_retirement_invalidates_before_any_waker_runs() {
        let owner = McpSubmissionRuntimeOwner::new();
        let runtime = owner.install(binding()).unwrap();
        let (waker, calls) = reentrant_waker(Callback::Wake, || {});
        let mut cancelled = Box::pin(runtime.cancellation.cancelled());
        assert!(
            cancelled
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        let deferred = owner.retire_deferred();
        assert!(runtime.live().is_err());
        assert_eq!(calls.calls(), 0);
        deferred.complete();
        assert_eq!(calls.calls(), 1);
        assert!(
            cancelled
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_ready()
        );
    }
}
