//! Inert ownership of a peer-selected, unsent application ID.

use super::RpcId;
use std::sync::Arc;

/// A non-clone reservation minted by the owning peer, not a submission grant.
///
/// Move this value into the exact typed request before permission preparation.
/// Dropping the last request owner makes its unsent peer slot reclaimable.
/// Destruction performs no I/O, callbacks, locking, or remote cancellation.
pub struct McpToolReservation {
    id: RpcId,
    state: Arc<()>,
}

impl McpToolReservation {
    #[must_use]
    pub const fn rpc_id(&self) -> &RpcId {
        &self.id
    }
}

impl std::fmt::Debug for McpToolReservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = &self.state;
        f.write_str("McpToolReservation { <redacted> }")
    }
}

/// The peer retains only a weak observer; request ownership never retains it.
#[derive(Default)]
#[cfg(any(test, target_os = "linux", target_os = "macos"))]
pub(crate) struct McpPendingToolReservation(Option<Pending>);

#[cfg(any(test, target_os = "linux", target_os = "macos"))]
enum Pending {
    Manual(RpcId),
    Leased {
        id: RpcId,
        state: std::sync::Weak<()>,
    },
}

#[cfg(any(test, target_os = "linux", target_os = "macos"))]
impl McpPendingToolReservation {
    pub(crate) fn manual(id: RpcId) -> Self {
        Self(Some(Pending::Manual(id)))
    }

    pub(crate) fn leased(id: RpcId) -> (Self, McpToolReservation) {
        let state = Arc::new(());
        let pending = Self(Some(Pending::Leased {
            id: id.clone(),
            state: Arc::downgrade(&state),
        }));
        (pending, McpToolReservation { id, state })
    }

    pub(crate) fn is_live(&self) -> bool {
        match &self.0 {
            None => false,
            Some(Pending::Manual(_)) => true,
            Some(Pending::Leased { state, .. }) => state.strong_count() != 0,
        }
    }

    pub(crate) fn matches(&self, id: &RpcId, lease: Option<&McpToolReservation>) -> bool {
        match (&self.0, lease) {
            (Some(Pending::Manual(expected)), None) => expected == id,
            (
                Some(Pending::Leased {
                    id: expected,
                    state,
                }),
                Some(lease),
            ) => expected == id && &lease.id == id && state.ptr_eq(&Arc::downgrade(&lease.state)),
            _ => false,
        }
    }

    /// Consume only the matching allocation. A rejected call cannot clear a
    /// different request, and a dropped old owner cannot clear its replacement.
    pub(crate) fn take(&mut self, id: &RpcId, lease: Option<&McpToolReservation>) -> Option<RpcId> {
        if !self.matches(id, lease) {
            return None;
        }
        match self.0.take()? {
            Pending::Manual(id) | Pending::Leased { id, .. } => Some(id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abandonment_releases_only_the_exact_weak_slot() {
        let (mut pending, lease) = McpPendingToolReservation::leased(RpcId::Integer(1));
        assert!(pending.is_live());
        assert!(pending.matches(lease.rpc_id(), Some(&lease)));
        assert!(!pending.matches(lease.rpc_id(), None));
        let (_, foreign) = McpPendingToolReservation::leased(RpcId::Integer(1));
        assert!(pending.take(foreign.rpc_id(), Some(&foreign)).is_none());
        assert!(pending.is_live());
        drop(lease);
        assert!(!pending.is_live());
        let (replacement, replacement_lease) = McpPendingToolReservation::leased(RpcId::Integer(2));
        pending = replacement;
        drop(foreign);
        assert!(pending.is_live());
        assert_eq!(
            pending.take(replacement_lease.rpc_id(), Some(&replacement_lease)),
            Some(RpcId::Integer(2))
        );
        assert!(!pending.is_live());
        assert!(
            pending
                .take(replacement_lease.rpc_id(), Some(&replacement_lease))
                .is_none()
        );
    }

    #[test]
    fn manual_compatibility_never_accepts_a_foreign_lease() {
        let id = RpcId::Integer(7);
        let mut pending = McpPendingToolReservation::manual(id.clone());
        let (_, foreign) = McpPendingToolReservation::leased(id.clone());
        assert!(pending.is_live());
        assert!(pending.take(&id, Some(&foreign)).is_none());
        assert!(pending.take(&RpcId::Integer(8), None).is_none());
        assert_eq!(pending.take(&id, None), Some(id));
        assert!(!pending.is_live());
    }
}
