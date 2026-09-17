//! One explicit host catalog, separate from each principal's model selection.
use super::{Arc, Mutex, NativeModelCatalog, RuntimeState};

/// Contains validated immutable data only, never a host, runtime, or fetcher.
/// Replacement retains one current catalog; admitted turns keep their original
/// resolved capabilities independently of later publication.
#[derive(Clone, Default)]
pub(crate) struct SharedModelCatalog(Arc<Mutex<Option<Arc<NativeModelCatalog>>>>);

pub(super) struct ManagedModelCatalog {
    source: SharedModelCatalog,
    publisher: bool,
}

#[cfg(feature = "ai-gateway-http")]
impl super::NativeConversationRuntime {
    /// Factory-only binding before the new runtime is handed to an owner.
    /// Foregrounds publish explicit host observations only after activation;
    /// children observe those snapshots without changing sibling selections.
    pub(crate) fn with_managed_model_catalog(
        self,
        source: SharedModelCatalog,
        foreground: bool,
    ) -> Self {
        {
            let mut state = self.state.lock().expect("runtime state poisoned");
            state.managed_catalog = Some(ManagedModelCatalog {
                source,
                publisher: foreground,
            });
        }
        self
    }
}

impl RuntimeState {
    pub(super) fn catalog_snapshot(&self) -> Option<Arc<NativeModelCatalog>> {
        self.catalog.clone().or_else(|| {
            self.managed_catalog.as_ref().and_then(|binding| {
                binding
                    .source
                    .0
                    .lock()
                    .expect("shared model catalog poisoned")
                    .clone()
            })
        })
    }

    /// Caller serializes against this principal's setters and drops the previous
    /// allocation outside both locks. No source operation acquires runtime state.
    pub(super) fn publish_catalog(&self) -> Option<Arc<NativeModelCatalog>> {
        let binding = self.managed_catalog.as_ref().filter(|b| b.publisher)?;
        let catalog = self.catalog.as_ref()?;
        binding
            .source
            .0
            .lock()
            .expect("shared model catalog poisoned")
            .replace(catalog.clone())
    }
}
