use std::collections::BTreeMap;
use std::fmt;

use super::{ACP_MAX_PENDING_REQUESTS, AcpId, AcpProtocolError};

/// Non-authoritative correlation labels. Native owners must separately retain
/// the actual session/incarnation, turn, permission and continuation custody.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AcpScope {
    pub session: u64,
    pub turn: u64,
    pub operation: u64,
    pub round: u32,
}

impl fmt::Debug for AcpScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AcpScope(<redacted>)")
    }
}

/// A maximum of 32 outbound requests, with non-reused IDs for this connection.
/// The driver must not replace this table while the connection remains alive.
#[derive(Default)]
pub struct AcpPendingRequests {
    next: u64,
    entries: BTreeMap<AcpId, AcpScope>,
}

impl fmt::Debug for AcpPendingRequests {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcpPendingRequests")
            .field("pending", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl AcpPendingRequests {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Reserves a connection-unique host ID; failed admission acquires nothing.
    ///
    /// # Errors
    /// Rejects capacity exhaustion and identifier wraparound.
    pub fn reserve(&mut self, scope: AcpScope) -> Result<AcpId, AcpProtocolError> {
        if self.entries.len() >= ACP_MAX_PENDING_REQUESTS {
            return Err(AcpProtocolError::PendingLimit);
        }
        let next = self
            .next
            .checked_add(1)
            .ok_or(AcpProtocolError::IdentifierExhausted)?;
        let id = AcpId::String(format!("machine-god:{next}"));
        self.entries.insert(id.clone(), scope);
        self.next = next;
        Ok(id)
    }

    /// Returns the retained correlation label without settling a request.
    #[must_use]
    pub fn scope(&self, id: &AcpId) -> Option<AcpScope> {
        self.entries.get(id).copied()
    }

    /// Settles only an exact ID and complete scope match. Mismatches do not
    /// remove the valid pending request or transfer any native custody.
    ///
    /// # Errors
    /// Rejects unknown, duplicate, invalidated or foreign-scope responses.
    pub fn complete(&mut self, id: &AcpId, scope: AcpScope) -> Result<AcpScope, AcpProtocolError> {
        let actual = self
            .entries
            .get(id)
            .ok_or(AcpProtocolError::UnknownResponse)?;
        if *actual != scope {
            return Err(AcpProtocolError::StaleResponse);
        }
        self.entries
            .remove(id)
            .ok_or(AcpProtocolError::UnknownResponse)
    }

    /// Invalidates a session incarnation and returns IDs so the native driver
    /// can settle the corresponding owned waiters before replacing that owner.
    #[must_use]
    pub fn invalidate_session(&mut self, session: u64) -> Vec<(AcpId, AcpScope)> {
        self.invalidate(|scope| scope.session == session)
    }

    /// Invalidates one turn without disturbing another session's equal labels.
    #[must_use]
    pub fn invalidate_turn(&mut self, session: u64, turn: u64) -> Vec<(AcpId, AcpScope)> {
        self.invalidate(|scope| scope.session == session && scope.turn == turn)
    }

    /// Removes all pending IDs, preserving the monotonic connection sequence.
    #[must_use]
    pub fn clear(&mut self) -> Vec<(AcpId, AcpScope)> {
        std::mem::take(&mut self.entries).into_iter().collect()
    }

    fn invalidate(&mut self, predicate: impl Fn(&AcpScope) -> bool) -> Vec<(AcpId, AcpScope)> {
        let ids: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, scope)| predicate(scope))
            .map(|(id, _)| id.clone())
            .collect();
        ids.into_iter()
            .filter_map(|id| self.entries.remove(&id).map(|scope| (id, scope)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhausted_identifiers_never_wrap_or_acquire_entries() {
        let mut pending = AcpPendingRequests {
            next: u64::MAX,
            entries: BTreeMap::new(),
        };
        let scope = AcpScope {
            session: 1,
            turn: 2,
            operation: 3,
            round: 4,
        };
        assert_eq!(
            pending.reserve(scope),
            Err(AcpProtocolError::IdentifierExhausted)
        );
        assert!(pending.is_empty());
    }
}
