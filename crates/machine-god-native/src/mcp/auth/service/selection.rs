//! Explicit configuration/command custody, independent of credential issuance.

use super::state::{Entry, Inner, MAX_IDENTITIES, lock};
use super::{McpAuthError, McpAuthIdentity, NativeMcpAuthService, Result};
use std::sync::{Arc, atomic::Ordering};

const MAX_SELECTIONS: usize = 128;

/// One native-selected identity owner. No file, clock or network observation.
/// Unlike a lease, this also owns a slot for an actually missing credential.
pub(crate) struct McpAuthSelection {
    inner: Arc<Inner>,
    identity: McpAuthIdentity,
    group: Arc<()>,
}
impl std::fmt::Debug for McpAuthSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("McpAuthSelection { <redacted> }")
    }
}
impl NativeMcpAuthService {
    pub(crate) fn retain_identity(&self, identity: &McpAuthIdentity) -> Result<McpAuthSelection> {
        self.inner.prune_retired_selections();
        let mut state = lock(&self.inner.state);
        if state.closed {
            return Err(McpAuthError::Unavailable);
        }
        if state.selections == MAX_SELECTIONS
            || !state.entries.contains_key(identity) && state.entries.len() == MAX_IDENTITIES
        {
            return Err(McpAuthError::Limit);
        }
        let entry = state
            .entries
            .entry(identity.clone())
            .or_insert_with(Entry::new);
        entry.prune();
        if entry.slot.cutoff.load(Ordering::Acquire) {
            if entry.pending() {
                return Err(McpAuthError::Busy);
            }
            entry.renew();
        }
        let group = entry.selection.upgrade().unwrap_or_else(|| Arc::new(()));
        entry.selection = Arc::downgrade(&group);
        entry.selection_count += 1;
        entry.managed = true;
        state.selections += 1;
        drop(state);
        Ok(McpAuthSelection {
            inner: self.inner.clone(),
            identity: identity.clone(),
            group,
        })
    }
}
impl Drop for McpAuthSelection {
    fn drop(&mut self) {
        let slot = {
            let mut state = lock(&self.inner.state);
            state.selections -= 1;
            state.entries.get_mut(&self.identity).and_then(|entry| {
                if !entry.selection.ptr_eq(&Arc::downgrade(&self.group)) {
                    return None;
                }
                entry.selection_count -= 1;
                if entry.selection_count == 0 && !entry.slot.cutoff.swap(true, Ordering::AcqRel) {
                    Some(entry.slot.clone())
                } else {
                    None
                }
            })
        };
        if let Some(slot) = slot {
            self.inner.invalidate(&self.identity, &slot);
        }
        // Prune on admission/cleanup, without a detached watcher. Owner counts
        // settle under the mutex: concurrent Drops must not both observe the
        // other's still-live Arc field and miss the final cutoff.
    }
}
impl Inner {
    pub(super) fn prune_retired_selections(&self) {
        let removed = {
            let mut state = lock(&self.state);
            state
                .entries
                .extract_if(.., |_, entry| {
                    entry.managed
                        && entry.selection_count == 0
                        && entry.slot.cutoff.load(Ordering::Acquire)
                        && !entry.pending()
                })
                .map(|(_, entry)| entry)
                .collect::<Vec<_>>()
        };
        drop(removed);
    }
}
