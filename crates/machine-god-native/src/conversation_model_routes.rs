//! Bounded, incarnation-bound access to a conversation's current model selection.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
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
    identity: Arc<AtomicBool>,
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
        let (source, identity) = {
            let entries = self.entries.lock().expect("model routes poisoned");
            let entry = entries.iter().find(|entry| {
                entry.session == context.session_id
                    && entry.incarnation == context.session_incarnation_id
            })?;
            (entry.source.upgrade()?, Arc::clone(&entry.identity))
        };
        // Never call a source or release its last strong reference under the
        // routing mutex. Runtime mutation needs only its own state mutex.
        if !identity.load(Ordering::Acquire) {
            return None;
        }
        let model = source.current_model();
        identity.load(Ordering::Acquire).then_some(model)
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
        let identity = Arc::new(AtomicBool::new(true));
        entries.push(Entry {
            identity: Arc::clone(&identity),
            session: session.clone(),
            incarnation: incarnation.clone(),
            source,
        });
        Ok(Arc::new(ModelRouteRegistration {
            routes: Arc::clone(self),
            session,
            incarnation,
            identity,
        }))
    }
}

pub(crate) struct ModelRouteRegistration {
    routes: Arc<NativeConversationModelRoutes>,
    session: SessionId,
    incarnation: SessionIncarnationId,
    identity: Arc<AtomicBool>,
}

impl ModelRouteRegistration {
    pub(crate) fn retire(&self) {
        self.identity.store(false, Ordering::Release);
        let removed = {
            let mut entries = self.routes.entries.lock().expect("model routes poisoned");
            entries
                .iter()
                .position(|entry| {
                    entry.session == self.session
                        && entry.incarnation == self.incarnation
                        && Arc::ptr_eq(&entry.identity, &self.identity)
                })
                .map(|index| entries.swap_remove(index))
        };
        drop(removed);
    }
}

impl Drop for ModelRouteRegistration {
    fn drop(&mut self) {
        self.retire();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Source {
        value: &'static str,
        retire: Mutex<Option<Arc<ModelRouteRegistration>>>,
    }
    impl CurrentModel for Source {
        fn current_model(&self) -> String {
            let registration = self.retire.lock().unwrap().take();
            if let Some(registration) = registration {
                registration.retire();
            }
            self.value.to_owned()
        }
    }
    fn context() -> ToolContext {
        ToolContext {
            session_id: SessionId::new("model-session").unwrap(),
            session_incarnation_id: SessionIncarnationId::new("same-incarnation").unwrap(),
            turn_id: machine_god_core::TurnId::new("turn-1").unwrap(),
            call_id: machine_god_core::ToolCallId::new("call").unwrap(),
        }
    }
    #[test]
    fn retained_registration_retirement_and_drop_cannot_remove_replacement() {
        let routes = Arc::new(NativeConversationModelRoutes::new());
        let context = context();
        let source: Arc<dyn CurrentModel> = Arc::new(Source {
            value: "old",
            retire: Mutex::new(None),
        });
        let old = routes
            .register(
                context.session_id.clone(),
                context.session_incarnation_id.clone(),
                Arc::downgrade(&source),
            )
            .unwrap();
        assert_eq!(routes.snapshot(&context).as_deref(), Some("old"));
        old.retire();
        assert!(routes.snapshot(&context).is_none());
        let source: Arc<dyn CurrentModel> = Arc::new(Source {
            value: "new",
            retire: Mutex::new(None),
        });
        let replacement = routes
            .register(
                context.session_id.clone(),
                context.session_incarnation_id.clone(),
                Arc::downgrade(&source),
            )
            .unwrap();
        old.retire();
        drop(old);
        assert_eq!(routes.snapshot(&context).as_deref(), Some("new"));
        drop(replacement);
        assert!(routes.snapshot(&context).is_none());
    }
    #[test]
    fn reentrant_source_retirement_is_unlocked_and_rejects_inflight_snapshot() {
        let routes = Arc::new(NativeConversationModelRoutes::new());
        let context = context();
        let source = Arc::new(Source {
            value: "must not escape",
            retire: Mutex::new(None),
        });
        let erased: Arc<dyn CurrentModel> = source.clone();
        let registration = routes
            .register(
                context.session_id.clone(),
                context.session_incarnation_id.clone(),
                Arc::downgrade(&erased),
            )
            .unwrap();
        *source.retire.lock().unwrap() = Some(registration);
        assert!(routes.snapshot(&context).is_none());
    }
}
