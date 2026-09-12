//! Peer ownership is independent of the finite deadline of each operation.

use std::time::Instant;

/// Trusted native host selection, not authority received from an MCP server.
/// Owner-controlled peers remain cancellable and every request/startup attempt
/// still has its own finite deadline. An explicit expiry only narrows ownership.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpPeerLifetime {
    OwnerControlled,
    Until(Instant),
}

impl McpPeerLifetime {
    #[must_use]
    pub const fn deadline(self) -> Option<Instant> {
        match self {
            Self::OwnerControlled => None,
            Self::Until(deadline) => Some(deadline),
        }
    }

    #[must_use]
    pub fn constrain(self, operation_deadline: Instant) -> Instant {
        self.deadline().map_or(operation_deadline, |deadline| {
            deadline.min(operation_deadline)
        })
    }

    #[must_use]
    pub fn is_expired(self, now: Instant) -> bool {
        self.deadline().is_some_and(|deadline| now >= deadline)
    }
}

impl From<Instant> for McpPeerLifetime {
    fn from(deadline: Instant) -> Self {
        Self::Until(deadline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn ownership_never_extends_an_operation_or_a_selected_expiry() {
        let now = Instant::now();
        let later = now + Duration::from_secs(10);
        let owner = McpPeerLifetime::OwnerControlled;
        assert_eq!(owner.deadline(), None);
        assert!(!owner.is_expired(later));
        assert_eq!(owner.constrain(now), now);
        let expiring = McpPeerLifetime::from(now);
        assert_eq!(expiring.deadline(), Some(now));
        assert!(expiring.is_expired(now));
        assert_eq!(expiring.constrain(later), now);
        assert_eq!(McpPeerLifetime::Until(later).constrain(now), now);
    }
}
