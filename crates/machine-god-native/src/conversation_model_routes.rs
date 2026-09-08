//! Bounded, incarnation-bound access to a conversation's current model selection.

use std::fmt;
use std::sync::{Arc, Mutex, Weak};

use machine_god_core::{SessionId, SessionIncarnationId, ToolContext};

pub const MAX_NATIVE_CONVERSATION_MODEL_ROUTES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeConversationModelRouteError {
    Duplicate,
    Capacity,
}

impl fmt::Display for NativeConversationModelRouteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("conversation model route unavailable")
    }
}

impl std::error::Error for NativeConversationModelRouteError {}

pub(crate) trait CurrentModel: Send + Sync {
    fn current_model(&self) -> String;
}

struct Entry {
    session: SessionId,
    incarnation: SessionIncarnationId,
    source: Weak<dyn CurrentModel>,
}

/// Explicit shared routing authority for selected-model secondary workers.
/// It owns no conversation, provider, filesystem or ambient process selection.
#[derive(Default)]
pub struct NativeConversationModelRoutes {
    entries: Mutex<Vec<Entry>>,
}

impl fmt::Debug for NativeConversationModelRoutes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeConversationModelRoutes { .. }")
    }
}

impl NativeConversationModelRoutes {
    /// Creates an empty bounded routing table without performing I/O.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Captures the current selected model for this exact live incarnation.
    /// Unknown or retired incarnations return `None`, never a fallback model.
    /// The returned owned value cannot change during a later capacity wait.
    ///
    /// # Panics
    /// Panics if an earlier panic poisoned a routing or runtime mutex.
    #[must_use]
    pub fn snapshot(&self, context: &ToolContext) -> Option<String> {
        let source = {
            let entries = self.entries.lock().expect("model routes poisoned");
            entries
                .iter()
                .find(|entry| {
                    entry.session == context.session_id
                        && entry.incarnation == context.session_incarnation_id
                })?
                .source
                .upgrade()?
        };
        // Never call a source or release its last strong reference under the
        // routing mutex. Runtime mutation needs only its own state mutex.
        Some(source.current_model())
    }

    pub(crate) fn register(
        self: &Arc<Self>,
        session: SessionId,
        incarnation: SessionIncarnationId,
        source: Weak<dyn CurrentModel>,
    ) -> Result<Arc<ModelRouteRegistration>, NativeConversationModelRouteError> {
        let mut entries = self.entries.lock().expect("model routes poisoned");
        if entries
            .iter()
            .any(|entry| entry.session == session && entry.incarnation == incarnation)
        {
            return Err(NativeConversationModelRouteError::Duplicate);
        }
        if entries.len() == MAX_NATIVE_CONVERSATION_MODEL_ROUTES {
            return Err(NativeConversationModelRouteError::Capacity);
        }
        entries.push(Entry {
            session: session.clone(),
            incarnation: incarnation.clone(),
            source,
        });
        Ok(Arc::new(ModelRouteRegistration {
            routes: Arc::clone(self),
            session,
            incarnation,
        }))
    }
}

pub(crate) struct ModelRouteRegistration {
    routes: Arc<NativeConversationModelRoutes>,
    session: SessionId,
    incarnation: SessionIncarnationId,
}

impl Drop for ModelRouteRegistration {
    fn drop(&mut self) {
        let removed = {
            let mut entries = self.routes.entries.lock().expect("model routes poisoned");
            entries
                .iter()
                .position(|entry| {
                    entry.session == self.session && entry.incarnation == self.incarnation
                })
                .map(|index| entries.swap_remove(index))
        };
        drop(removed);
    }
}
