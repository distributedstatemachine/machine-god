//! Inert ownership of a peer-selected, unsent application ID.

use super::RpcId;
use std::sync::Arc;

/// Maximum unsent, independently owned tool IDs on one serialized peer.
pub const MAX_MCP_PEER_RESERVATIONS: usize = 64;

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
pub(crate) struct McpPendingToolReservation(Vec<Pending>);

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
        Self(vec![Pending::Manual(id)])
    }

    #[cfg(test)]
    pub(crate) fn leased(id: RpcId) -> (Self, McpToolReservation) {
        let mut pending = Self::default();
        let lease = pending.reserve(id).expect("empty reservation set");
        (pending, lease)
    }

    pub(crate) fn has_capacity(&mut self) -> bool {
        self.0.retain(Pending::is_live);
        !self.blocks_control() && self.0.len() < MAX_MCP_PEER_RESERVATIONS
    }

    pub(crate) fn reserve(&mut self, id: RpcId) -> Option<McpToolReservation> {
        if !self.has_capacity() || self.0.iter().any(|pending| pending.id() == &id) {
            return None;
        }
        let state = Arc::new(());
        self.0.push(Pending::Leased {
            id: id.clone(),
            state: Arc::downgrade(&state),
        });
        Some(McpToolReservation { id, state })
    }

    pub(crate) fn is_live(&self) -> bool {
        self.0.iter().any(Pending::is_live)
    }

    pub(crate) fn blocks_control(&self) -> bool {
        self.0
            .iter()
            .any(|pending| matches!(pending, Pending::Manual(_)))
    }

    pub(crate) fn discard_manual(&mut self) {
        self.0
            .retain(|pending| !matches!(pending, Pending::Manual(_)));
    }

    #[cfg(any(test, feature = "mcp-http"))]
    pub(crate) fn matches(&self, id: &RpcId, lease: Option<&McpToolReservation>) -> bool {
        self.0.iter().any(|pending| pending.matches(id, lease))
    }

    /// Consume only the matching allocation. A rejected call cannot clear a
    /// different request, and a dropped old owner cannot clear its replacement.
    pub(crate) fn take(&mut self, id: &RpcId, lease: Option<&McpToolReservation>) -> Option<RpcId> {
        let index = self
            .0
            .iter()
            .position(|pending| pending.matches(id, lease))?;
        match self.0.swap_remove(index) {
            Pending::Manual(id) | Pending::Leased { id, .. } => Some(id),
        }
    }
}

#[cfg(any(test, target_os = "linux", target_os = "macos"))]
impl Pending {
    fn id(&self) -> &RpcId {
        match self {
            Self::Manual(id) | Self::Leased { id, .. } => id,
        }
    }

    fn is_live(&self) -> bool {
        match self {
            Self::Manual(_) => true,
            Self::Leased { state, .. } => state.strong_count() != 0,
        }
    }

    fn matches(&self, id: &RpcId, lease: Option<&McpToolReservation>) -> bool {
        match (self, lease) {
            (Self::Manual(expected), None) => expected == id,
            (
                Self::Leased {
                    id: expected,
                    state,
                },
                Some(lease),
            ) => expected == id && &lease.id == id && state.ptr_eq(&Arc::downgrade(&lease.state)),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiple_leases_are_bounded_independent_and_do_not_block_controls() {
        let mut pending = McpPendingToolReservation::default();
        let mut leases: Vec<_> = (0..64)
            .map(|id| pending.reserve(RpcId::Integer(id)).unwrap())
            .collect();
        assert!(!pending.blocks_control());
        assert!(pending.reserve(RpcId::Integer(64)).is_none());
        pending.discard_manual();
        assert!(!pending.has_capacity());
        drop(leases.remove(0));
        let last = pending.reserve(RpcId::Integer(64)).unwrap();
        assert!(pending.reserve(RpcId::Integer(64)).is_none());
        let selected = leases.remove(20);
        assert_eq!(
            pending.take(selected.rpc_id(), Some(&selected)),
            Some(RpcId::Integer(21))
        );
        assert!(pending.has_capacity());
        for lease in &leases {
            assert!(pending.matches(lease.rpc_id(), Some(lease)));
        }
        assert!(pending.matches(last.rpc_id(), Some(&last)));
        let mut manual = McpPendingToolReservation::manual(RpcId::Integer(1));
        assert!(manual.blocks_control());
        assert!(manual.reserve(RpcId::Integer(2)).is_none());
        manual.discard_manual();
        assert!(manual.has_capacity());
    }

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
